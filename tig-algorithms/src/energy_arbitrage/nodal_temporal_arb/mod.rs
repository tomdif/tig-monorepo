use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tig_challenges::energy_arbitrage::{constants, Challenge, State};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Hyperparameters {
    pub horizon_steps: usize,
    pub urgency_gain: f64,
    pub rt_z_gain: f64,
    pub min_window_std: f64,
    pub action_sharpness: f64,
    pub profit_floor_shrink: f64,
}

impl Default for Hyperparameters {
    fn default() -> Self {
        Self {
            horizon_steps: 192,
            urgency_gain: 0.5,
            rt_z_gain: 1.5,
            min_window_std: 1.0,
            action_sharpness: 0.5,
            profit_floor_shrink: 0.95,
        }
    }
}

pub fn help() {
    println!(
        "nodal_temporal_arb: per-node temporal arbitrage. Targets SOC by price-rank \
in a per-node DA window, modulates with standardized RT deviation, projects to \
PTDF feasibility, and enforces non-negative cumulative profit. \
Hyperparameters: horizon_steps, urgency_gain, rt_z_gain, min_window_std, \
action_sharpness, profit_floor_shrink."
    );
}

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&tig_challenges::energy_arbitrage::Solution) -> Result<()>,
    hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    let hp = parse_hyperparameters(hyperparameters);
    let solution = challenge.grid_optimize(&|c, s| policy(c, s, &hp))?;
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

fn price_rank(window: &[f64], p_now: f64) -> f64 {
    if window.is_empty() {
        return 0.5;
    }
    let strictly_below = window.iter().filter(|&&p| p < p_now).count() as f64;
    let equal = window.iter().filter(|&&p| (p - p_now).abs() <= EPS).count() as f64;
    // Midrank within ties; output in (0, 1).
    (strictly_below + 0.5 * equal) / window.len() as f64
}

pub fn policy(challenge: &Challenge, state: &State, hp: &Hyperparameters) -> Result<Vec<f64>> {
    let t = state.time_step;
    let h = challenge.num_steps;
    let k = hp.horizon_steps.min(h.saturating_sub(t));
    let da = &challenge.market.day_ahead_prices;

    if t >= da.len() {
        return Err(anyhow!("DA prices missing for step {}", t));
    }

    let mut action = vec![0.0; challenge.num_batteries];

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
        let (mean, std) = window_stats(&window);
        if !std.is_finite() || std < hp.min_window_std {
            continue;
        }

        let p_now = da[t][n];
        let rank = price_rank(&window, p_now);

        // Target SOC fraction: extreme prices → extreme target.
        // p_now is the lowest in the window (rank≈0) → target_frac=1 (full).
        // p_now is the highest in the window (rank≈1) → target_frac=0 (empty).
        let target_frac = 1.0 - rank;
        let denom = (battery.soc_max_mwh - battery.soc_min_mwh).max(EPS);
        let cur_frac = ((state.socs[i] - battery.soc_min_mwh) / denom).clamp(0.0, 1.0);

        // Urgency: positive → discharge (current SOC above target), negative → charge.
        let urgency = (cur_frac - target_frac) * hp.urgency_gain;

        // Standardized RT deviation. Bounded via tanh: large RT spikes saturate to ±1.
        let _ = mean; // mean unused; rank captures position
        let z_rt = (state.rt_prices[n] - p_now) / std.max(EPS);
        let rt_signal = (z_rt * hp.rt_z_gain * 0.5).tanh();

        // Combined signal in [-1, 1]; sharpness controls bang-bang vs continuous.
        let signal = ((urgency + rt_signal) / hp.action_sharpness.max(0.1)).tanh();

        // Map signed signal to MW within available bounds.
        // signal > 0 → discharge into hi (≥0); signal < 0 → charge toward lo (≤0).
        let raw = if signal >= 0.0 {
            signal * hi
        } else {
            -signal * lo
        };

        action[i] = raw.clamp(lo, hi);
    }

    let action = enforce_flow_feasibility(challenge, state, action)?;
    Ok(enforce_profit_floor(challenge, state, action, hp.profit_floor_shrink))
}

// Important! Do not include any tests in this file, it will result in your submission being rejected
