# TIG Code Submission

## Submission Details

* **Challenge Name:** energy_arbitrage
* **Algorithm Name:** nodal_temporal_arb
* **Copyright:** 2026 [submitter to fill]
* **Identity of Submitter:** [submitter to fill]
* **Identity of Creator of Algorithmic Method:** [submitter to fill]
* **Unique Algorithm Identifier (UAI):** null

## Method

For each battery, the policy solves a finite-horizon dynamic-programming problem
over the full simulation horizon and uses the resulting value function to drive
a per-step action choice that combines planning (DA prices) with reactive
correction (realised RT prices and tail-jump detection).

### Per-battery dynamic programming

`compute_battery_dp` runs once per challenge, before the rollout begins. For
each battery it builds a value table `V[t][soc_idx]` by backward induction:

```
V[H][·] = 0
V[t][soc] = max_u [reward_DA(u, p_DA[t][n]) + V[t+1][soc(u)]]
reward_DA(u, p) = u·p·Δt − κ_tx·|u|·Δt − κ_deg·(|u|·Δt/E̅)^β
```

with `u` chosen on a discrete action grid and `soc(u)` snapped to the nearest
of `dp_soc_levels` discretised levels. The DP is computed against day-ahead
prices only; RT realisations are folded in at execution time.

### Per-step action selection

At each rollout step `t`, for every battery `i`:

1. **Tail-jump override.** Compute the standardised RT deviation
   `z_rt = (rt_prices[n] − DA[t][n]) / window_std`. When `|z_rt|` exceeds
   `jump_z_threshold`, commit a full-bound action in the direction of the
   deviation, bypassing the DP.
2. **DP-driven action.** Otherwise, search a finer action grid for the
   maximiser of
   `reward_RT(u, rt_prices[n]) + V[t+1][soc(u)]`,
   substituting the *realised* RT price into the immediate-reward term while
   relying on the DP value function to summarise the future.

### Cross-battery feasibility

After all batteries pick their candidate actions, the vector is projected onto
PTDF feasibility (greedy line softening + binary-scale fallback, copied from the
published `greedy` baseline) and then expanded by the largest scalar α ≥ 1
that keeps both line and per-battery bounds. Finally a profit-floor loop
(adapted from the published `conservative` baseline) shrinks magnitudes if the
running cumulative profit would otherwise go negative.

### What's different from the public baselines

`greedy` looks at node 0 as a proxy and bang-bangs on a fixed price gap;
`conservative` looks at the per-step cross-node average and bang-bangs on a
±5% threshold. Neither references SOC, neither plans across the horizon, and
neither uses the realised RT price. This policy uses all three: per-node DA
prices for planning, realised RT for execution, and the value function carries
the SOC/horizon trade-off automatically.

## Hyperparameters

| Field | Default | Meaning |
|---|---|---|
| `horizon_steps` | 192 | Window cap for the std/jump statistics (auto-clipped to remaining steps). |
| `min_window_std` | 0.0 | Skip trading when the window price std is below this; `0.0` means never skip. |
| `profit_floor_shrink` | 0.95 | Per-iter shrink factor when cumulative profit floor binds. |
| `jump_z_threshold` | 4.0 | Standardised-RT magnitude that triggers full-bound bang override. |
| `dp_soc_levels` | 24 | SOC discretisation count for the per-battery DP. |
| `dp_action_levels` | 21 | Action-grid resolution for both DP backward-induction and online selection. |

Legacy fields (`urgency_gain`, `rt_z_gain`, `action_sharpness`) are retained on
the struct for serde compatibility with prior submissions but are not used by
the current policy.

The defaults were selected by sweep on 50 challenge instances (5 scenarios ×
10 seeds), scoring with TIG's per-nonce quality formula
`((profit − baseline) / |baseline|).clamp(-10, 10)`. Under these settings the
policy beats `max(greedy, conservative)` on **45 of 50 instances** (8.96×
aggregate profit, $10.29M vs $1.15M baseline) with mean per-scenario clamped
quality of:

| Scenario | mean quality (clamp ±10) |
|---|---|
| BASELINE | +2.34 |
| CONGESTED | +5.07 |
| MULTIDAY | +8.20 |
| DENSE | +9.17 |
| CAPSTONE | +8.56 |
| **total / max 50** | **33.34** |

This is +1.12 over the prior smooth-rank policy (32.22), with the largest gain
on CONGESTED (+0.75) where DP planning across line-tight steps adds the most
value over per-step heuristics.

## References and Acknowledgments

### Code References

* Feasibility projection (greedy line softening + binary-scale fallback)
  adapted from the published `energy_arbitrage` greedy baseline in
  `tig-challenges/src/energy_arbitrage/baselines/greedy.rs`.
* Profit-floor shrink loop adapted from the published
  `energy_arbitrage` conservative baseline in
  `tig-challenges/src/energy_arbitrage/baselines/conservative.rs`.

## License

The files in this folder are under the following licenses:

* TIG Benchmarker Outbound License
* TIG Commercial License
* TIG Inbound Game License
* TIG Innovator Outbound Game License
* TIG Open Data License
* TIG THV Game License

Copies of the licenses can be obtained at:
https://github.com/tig-foundation/tig-monorepo/tree/main/docs/licenses
