use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tig_challenges::energy_arbitrage::{constants, Battery, Challenge, State};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Hyperparameters {
    pub horizon_steps: usize,
    pub min_window_std: f64,
    pub profit_floor_shrink: f64,
    pub jump_z_threshold: f64,
    pub dp_soc_levels: usize,
    pub dp_action_levels: usize,
    /// Legacy smooth-policy parameters retained for serde compat with prior submissions.
    pub urgency_gain: f64,
    pub rt_z_gain: f64,
    pub action_sharpness: f64,
}

impl Default for Hyperparameters {
    fn default() -> Self {
        Self {
            horizon_steps: 192,
            min_window_std: 0.0,
            profit_floor_shrink: 0.95,
            jump_z_threshold: 4.0,
            dp_soc_levels: 24,
            dp_action_levels: 21,
            urgency_gain: 1.0,
            rt_z_gain: 1.0,
            action_sharpness: 0.30,
        }
    }
}

pub fn help() {
    println!(
        "nodal_temporal_arb: per-battery dynamic-programming policy. \
For each battery, computes a backward DP value table V[t][soc] using day-ahead \
prices at the battery's node. At each step, picks the action that maximises \
`reward(u, RT_now) + V[t+1][soc(u)]`. Tail RT jumps trigger a full-bound bang \
override; PTDF feasibility is enforced via greedy line softening + symmetric \
expansion; cumulative profit floor matches the published `conservative` baseline. \
Hyperparameters: horizon_steps, min_window_std, profit_floor_shrink, \
jump_z_threshold, dp_soc_levels, dp_action_levels."
    );
}

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&tig_challenges::energy_arbitrage::Solution) -> Result<()>,
    hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    let hp = parse_hyperparameters(hyperparameters);
    // Precompute per-battery DP value tables once for the whole rollout.
    let dp_tables: Vec<(Vec<Vec<f64>>, f64)> = challenge.batteries.iter()
        .map(|b| compute_battery_dp(challenge, b, hp.dp_soc_levels, hp.dp_action_levels))
        .collect();
    let solution = challenge.grid_optimize(&|c, s| policy(c, s, &hp, &dp_tables))?;
    save_solution(&solution)?;
    Ok(())
}

fn parse_hyperparameters(raw: &Option<Map<String, Value>>) -> Hyperparameters {
    raw.as_ref()
        .and_then(|m| serde_json::from_value(Value::Object(m.clone())).ok())
        .unwrap_or_default()
}

const MAX_FLOW_ADJUST_ITERS: usize = 64;
const GLOBAL_SCALE_BSEARCH_ITERS: usize = 32;
const PROFIT_FLOOR_MAX_ITERS: usize = 64;
const EPS: f64 = 1e-12;

#[derive(Clone, Copy)]
struct Violation {
    line: usize,
    flow: f64,
    amount: f64,
}

fn compute_flows(challenge: &Challenge, state: &State, action: &[f64]) -> Vec<f64> {
    let injections = challenge.compute_total_injections(state, action);
    (0..challenge.network.num_lines)
        .map(|l| {
            (0..challenge.network.num_nodes)
                .map(|k| challenge.network.ptdf[l][k] * injections[k])
                .sum::<f64>()
        })
        .collect()
}

fn most_violated_line(challenge: &Challenge, flows: &[f64]) -> Option<Violation> {
    let mut best: Option<Violation> = None;
    for (l, &flow) in flows.iter().enumerate() {
        let limit = challenge.network.flow_limits[l];
        let violation = flow.abs() - limit;
        if violation > constants::EPS_FLOW * limit {
            let candidate = Violation { line: l, flow, amount: violation };
            match best {
                Some(current) if candidate.amount <= current.amount => {}
                _ => best = Some(candidate),
            }
        }
    }
    best
}

fn is_flow_feasible(challenge: &Challenge, state: &State, action: &[f64]) -> bool {
    let flows = compute_flows(challenge, state, action);
    most_violated_line(challenge, &flows).is_none()
}

fn soften_most_violated_line(challenge: &Challenge, v: Violation, action: &mut [f64]) -> bool {
    let dir = v.flow.signum();
    if dir.abs() <= EPS {
        return false;
    }
    let mut idxs = Vec::new();
    let mut strength = 0.0;
    for (i, battery) in challenge.batteries.iter().enumerate() {
        let contrib = challenge.network.ptdf[v.line][battery.node] * action[i];
        let signed = dir * contrib;
        if signed > EPS {
            strength += signed;
            idxs.push(i);
        }
    }
    if idxs.is_empty() || strength <= EPS {
        return false;
    }
    let keep = (1.0 - v.amount / strength).clamp(0.0, 1.0);
    if (1.0 - keep).abs() <= EPS {
        return false;
    }
    for i in idxs {
        action[i] *= keep;
    }
    true
}

fn enforce_flow_feasibility(
    challenge: &Challenge,
    state: &State,
    mut action: Vec<f64>,
) -> Result<Vec<f64>> {
    for _ in 0..MAX_FLOW_ADJUST_ITERS {
        let flows = compute_flows(challenge, state, &action);
        let Some(v) = most_violated_line(challenge, &flows) else {
            return Ok(action);
        };
        if !soften_most_violated_line(challenge, v, &mut action) {
            break;
        }
    }
    if is_flow_feasible(challenge, state, &action) {
        return Ok(action);
    }
    let zero = vec![0.0; action.len()];
    if !is_flow_feasible(challenge, state, &zero) {
        return Err(anyhow!("Grid infeasible even at zero action"));
    }
    let base = action;
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..GLOBAL_SCALE_BSEARCH_ITERS {
        let mid = 0.5 * (low + high);
        let scaled: Vec<f64> = base.iter().map(|u| mid * u).collect();
        if is_flow_feasible(challenge, state, &scaled) {
            low = mid;
        } else {
            high = mid;
        }
    }
    Ok(base.into_iter().map(|u| low * u).collect())
}

/// After the projection, the action may be feasible but with slack on every line.
/// Find the largest scalar α ≥ 1 such that α * action is still feasible (PTDF + per-battery
/// bounds), and apply it. Captures any slack the greedy projection left on the table.
fn expand_to_feasibility_limit(challenge: &Challenge, state: &State, action: &[f64]) -> Vec<f64> {
    if action.iter().all(|u| u.abs() <= EPS) {
        return action.to_vec();
    }
    let mut alpha_max = f64::INFINITY;
    for (i, &u) in action.iter().enumerate() {
        if u.abs() <= EPS { continue; }
        let (lo, hi) = state.action_bounds[i];
        let bound = if u > 0.0 { hi / u } else { lo / u };
        if bound.is_finite() && bound < alpha_max { alpha_max = bound; }
    }
    if !alpha_max.is_finite() || alpha_max <= 1.0 + EPS {
        return action.to_vec();
    }
    let mut lo_a = 1.0;
    let mut hi_a = alpha_max.max(1.0);
    for _ in 0..GLOBAL_SCALE_BSEARCH_ITERS {
        let mid = 0.5 * (lo_a + hi_a);
        let scaled: Vec<f64> = action.iter().map(|u| mid * u).collect();
        if is_flow_feasible(challenge, state, &scaled) { lo_a = mid; } else { hi_a = mid; }
    }
    if lo_a <= 1.0 + EPS {
        return action.to_vec();
    }
    action.iter().enumerate()
        .map(|(i, u)| {
            let (lo, hi) = state.action_bounds[i];
            (lo_a * u).clamp(lo, hi)
        }).collect()
}

fn enforce_profit_floor(challenge: &Challenge, state: &State, mut action: Vec<f64>, shrink: f64) -> Vec<f64> {
    let mut profit = challenge.compute_profit(state, &action);
    if state.total_profit + profit >= 0.0 {
        return action;
    }
    let shrink = shrink.clamp(0.5, 0.999);
    for _ in 0..PROFIT_FLOOR_MAX_ITERS {
        if state.total_profit + profit >= 0.0 {
            return action;
        }
        action = action
            .into_iter()
            .map(|u| if u.abs() < EPS { 0.0 } else { u * shrink })
            .collect();
        profit = challenge.compute_profit(state, &action);
    }
    vec![0.0; action.len()]
}

fn window_stats(window: &[f64]) -> (f64, f64) {
    let n = window.len() as f64;
    if n <= 0.0 {
        return (0.0, 0.0);
    }
    let mean = window.iter().sum::<f64>() / n;
    let var = window.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

/// Compute per-battery value function V[t][soc_idx] via backward DP using the full
/// day-ahead price series at the battery's node. V[H] = 0; works back to t=0.
/// Returns (V_table, soc_step). The DP plans against DA prices only — the per-step
/// policy substitutes the realised RT price into the immediate-reward term.
fn compute_battery_dp(
    challenge: &Challenge,
    battery: &Battery,
    n_soc_levels: usize,
    n_actions: usize,
) -> (Vec<Vec<f64>>, f64) {
    let h = challenge.num_steps;
    let n = battery.node;
    let soc_min = battery.soc_min_mwh;
    let soc_max = battery.soc_max_mwh;
    let soc_step = (soc_max - soc_min) / (n_soc_levels - 1).max(1) as f64;
    let dt = constants::DELTA_T;
    let kappa_tx = constants::KAPPA_TX;
    let kappa_deg = constants::KAPPA_DEG;
    let beta_deg = constants::BETA_DEG;

    // Action grid spans [-power_charge, +power_discharge] linearly.
    let action_grid: Vec<f64> = (0..n_actions).map(|i| {
        let frac = (i as f64) / (n_actions - 1).max(1) as f64 * 2.0 - 1.0;
        if frac < 0.0 { frac * battery.power_charge_mw } else { frac * battery.power_discharge_mw }
    }).collect();

    let mut v = vec![vec![0.0_f64; n_soc_levels]; h + 1];
    for t in (0..h).rev() {
        let p_da = challenge.market.day_ahead_prices[t][n];
        for soc_idx in 0..n_soc_levels {
            let soc = soc_min + soc_idx as f64 * soc_step;
            let headroom = (soc_max - soc).max(0.0);
            let available = (soc - soc_min).max(0.0);
            let max_charge = (headroom / (battery.efficiency_charge.max(EPS) * dt))
                .min(battery.power_charge_mw).max(0.0);
            let max_discharge = (available * battery.efficiency_discharge / dt)
                .min(battery.power_discharge_mw).max(0.0);
            let lo = -max_charge;
            let hi = max_discharge;
            let mut best = f64::NEG_INFINITY;
            for &u_raw in &action_grid {
                let u = u_raw.clamp(lo, hi);
                let revenue = u * p_da * dt;
                let abs_u = u.abs();
                let tx = kappa_tx * abs_u * dt;
                let deg = kappa_deg * ((abs_u * dt) / battery.capacity_mwh).powf(beta_deg);
                let reward = revenue - tx - deg;
                let new_soc = battery.apply_action_to_soc(u, soc);
                let new_soc_idx = (((new_soc - soc_min) / soc_step.max(EPS)).round() as i32)
                    .max(0).min((n_soc_levels - 1) as i32) as usize;
                let value = reward + v[t + 1][new_soc_idx];
                if value > best { best = value; }
            }
            v[t][soc_idx] = if best.is_finite() { best } else { 0.0 };
        }
    }
    (v, soc_step)
}

pub fn policy(
    challenge: &Challenge,
    state: &State,
    hp: &Hyperparameters,
    dp_tables: &[(Vec<Vec<f64>>, f64)],
) -> Result<Vec<f64>> {
    let t = state.time_step;
    let h = challenge.num_steps;
    let k = hp.horizon_steps.min(h.saturating_sub(t));
    let da = &challenge.market.day_ahead_prices;
    if t >= da.len() {
        return Err(anyhow!("DA prices missing for step {}", t));
    }
    let mut action = vec![0.0_f64; challenge.num_batteries];
    let dt = constants::DELTA_T;
    let kappa_tx = constants::KAPPA_TX;
    let kappa_deg = constants::KAPPA_DEG;
    let beta_deg = constants::BETA_DEG;
    let n_actions = hp.dp_action_levels.max(3);

    for (i, battery) in challenge.batteries.iter().enumerate() {
        let n = battery.node;
        let (lo, hi) = state.action_bounds[i];
        if (hi - lo).abs() <= EPS {
            continue;
        }
        let end = (t + k).min(h);
        if end <= t {
            continue;
        }
        let window: Vec<f64> = (t..end).map(|s| da[s][n]).collect();
        let (_mean, std) = window_stats(&window);
        if !std.is_finite() || std < hp.min_window_std {
            continue;
        }
        let p_now = da[t][n];
        let z_rt = (state.rt_prices[n] - p_now) / std.max(EPS);

        // Tail-jump override
        if hp.jump_z_threshold > 0.0 && z_rt.abs() >= hp.jump_z_threshold {
            action[i] = if z_rt > 0.0 { hi } else { lo };
            continue;
        }

        // DP value-driven action selection: maximise reward(u, RT_now) + V[t+1][soc(u)]
        if i >= dp_tables.len() {
            continue;
        }
        let (ref v_tab, soc_step) = dp_tables[i];
        if t + 1 >= v_tab.len() {
            continue;
        }
        let v_next = &v_tab[t + 1];
        let n_soc_levels = v_next.len();
        let soc_min_i = battery.soc_min_mwh;
        let p_rt = state.rt_prices[n];
        let mut best_value = f64::NEG_INFINITY;
        let mut best_u = 0.0_f64;
        for j in 0..n_actions {
            let frac = (j as f64) / (n_actions - 1).max(1) as f64 * 2.0 - 1.0;
            let u_raw = if frac < 0.0 { frac * battery.power_charge_mw } else { frac * battery.power_discharge_mw };
            let u = u_raw.clamp(lo, hi);
            let revenue = u * p_rt * dt;
            let abs_u = u.abs();
            let tx = kappa_tx * abs_u * dt;
            let deg = kappa_deg * ((abs_u * dt) / battery.capacity_mwh).powf(beta_deg);
            let reward = revenue - tx - deg;
            let new_soc = battery.apply_action_to_soc(u, state.socs[i]);
            let new_soc_idx = (((new_soc - soc_min_i) / soc_step.max(EPS)).round() as i32)
                .max(0).min((n_soc_levels - 1) as i32) as usize;
            let value = reward + v_next[new_soc_idx];
            if value > best_value {
                best_value = value;
                best_u = u;
            }
        }
        action[i] = best_u;
    }

    let action = enforce_flow_feasibility(challenge, state, action)?;
    let action = expand_to_feasibility_limit(challenge, state, &action);
    Ok(enforce_profit_floor(challenge, state, action, hp.profit_floor_shrink))
}

// Important! Do not include any tests in this file, it will result in your submission being rejected
