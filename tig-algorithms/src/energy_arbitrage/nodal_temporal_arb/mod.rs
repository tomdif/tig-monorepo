use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tig_challenges::energy_arbitrage::{constants, Challenge, State};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Hyperparameters {
    pub horizon_steps: usize,
    pub charge_threshold: f64,
    pub discharge_threshold: f64,
    pub min_spread: f64,
    pub rt_blend: f64,
    pub rt_scale: f64,
}

impl Default for Hyperparameters {
    fn default() -> Self {
        Self {
            horizon_steps: 24,
            charge_threshold: 0.30,
            discharge_threshold: 0.70,
            min_spread: 5.0,
            rt_blend: 0.20,
            rt_scale: 50.0,
        }
    }
}

pub fn help() {
    println!(
        "nodal_temporal_arb: per-node temporal arbitrage with continuous actions, \
RT-price correction, and PTDF-aware feasibility projection. \
Hyperparameters: horizon_steps, charge_threshold, discharge_threshold, \
min_spread, rt_blend, rt_scale."
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
    let line = v.line;
    let dir = v.flow.signum();
    if dir.abs() <= EPS {
        return false;
    }
    let mut idxs = Vec::new();
    let mut strength = 0.0;
    for (i, battery) in challenge.batteries.iter().enumerate() {
        let contrib = challenge.network.ptdf[line][battery.node] * action[i];
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

        // Build per-node DA window: [t .. t+k] inclusive of current
        let end = (t + k).min(h);
        if end <= t {
            continue;
        }
        let mut p_min = f64::INFINITY;
        let mut p_max = f64::NEG_INFINITY;
        for s in t..end {
            let p = da[s][n];
            if p < p_min {
                p_min = p;
            }
            if p > p_max {
                p_max = p;
            }
        }
        let spread = p_max - p_min;
        if !spread.is_finite() || spread < hp.min_spread {
            continue;
        }
        let p_now = da[t][n];
        let norm = ((p_now - p_min) / spread).clamp(0.0, 1.0);

        // Continuous DA-driven action signal in [-1, 1]
        let mut signal = if norm < hp.charge_threshold {
            -((hp.charge_threshold - norm) / hp.charge_threshold.max(EPS))
        } else if norm > hp.discharge_threshold {
            (norm - hp.discharge_threshold) / (1.0 - hp.discharge_threshold).max(EPS)
        } else {
            0.0
        };

        // RT correction: deviation of current RT from DA forecast at this node.
        // Positive deviation => discharge harder; negative => charge harder.
        let rt_dev = state.rt_prices[n] - p_now;
        let rt_signal = (rt_dev / hp.rt_scale).clamp(-1.0, 1.0);
        signal = (1.0 - hp.rt_blend) * signal + hp.rt_blend * rt_signal;
        let signal = signal.clamp(-1.0, 1.0);

        // Map signal to MW: negative → charge bound, positive → discharge bound
        let raw = if signal < 0.0 {
            -signal * (-battery.power_charge_mw)
        } else {
            signal * battery.power_discharge_mw
        };

        action[i] = raw.clamp(lo, hi);
    }

    enforce_flow_feasibility(challenge, state, action)
}

// Important! Do not include any tests in this file, it will result in your submission being rejected
