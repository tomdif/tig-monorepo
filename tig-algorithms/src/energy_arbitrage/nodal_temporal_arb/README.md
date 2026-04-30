# TIG Code Submission

## Submission Details

* **Challenge Name:** energy_arbitrage
* **Algorithm Name:** nodal_temporal_arb
* **Copyright:** 2026 tomdif
* **Identity of Submitter:** tomdif
* **Identity of Creator of Algorithmic Method:** tomdif
* **Unique Algorithm Identifier (UAI):** null

## Method

A per-battery dynamic-programming value function combined with PTDF-aware
coordinate refinement at each step.

### Per-battery dynamic programming

`compute_battery_dp` runs once per challenge before the rollout begins. For
each battery it builds a value table `V[t][soc_idx]` by backward induction
against the day-ahead price series at the battery's node:

```
V[H][·] = 0
V[t][soc] = max_u [reward(u, p_DA[t]) + V[t+1][soc(u)]]
reward(u, p) = u·p·Δt − κ_tx·|u|·Δt − κ_deg·(|u|·Δt/E̅)^β
```

with `u` on a discrete action grid (`dp_action_levels` points spanning
`[-power_charge, +power_discharge]`) and `soc(u)` snapped to the nearest of
`dp_soc_levels` discretised levels.

### Per-step action selection

At each rollout step `t`, for every battery `i`:

1. **Tail-jump override.** Compute the standardised RT deviation
   `z_rt = (rt_prices[n] − DA[t][n]) / window_std`. When `|z_rt|` exceeds
   `jump_z_threshold`, commit a full-bound action in the direction of the
   deviation, bypassing the DP.
2. **DP-driven action.** Otherwise pick the action `u` that maximises
   `reward(u, rt_prices[n]) + V[t+1][soc(u)]`, substituting the *realised* RT
   price into the immediate-reward term while letting the value function carry
   the SOC/horizon trade-off.

### Cross-battery feasibility and refinement

After all batteries propose actions, the vector goes through three stages:

1. **PTDF feasibility projection.** Greedy line-by-line softening + binary-scale
   fallback (copied from the published `greedy` baseline).
2. **Symmetric expansion.** Find the largest scalar α ≥ 1 such that α·action is
   still feasible (PTDF + per-battery bounds), and apply it.
3. **Coordinate refinement.** For up to four passes, iterate over each battery
   and replace its action with the action-grid point that maximises
   `reward(u, rt_prices[n]) + V[t+1][soc(u)]` subject to the per-line flow
   constraint `|flow_l + ptdf[l][node]·(u − u_old)| ≤ flow_limit_l`. This
   captures Pareto-improving moves that the projection's greedy shrink cannot
   make — including *reducing* magnitudes where the initial DP choice was
   over-aggressive given multi-battery line coupling. Stops early when no
   battery changes in a full pass.
4. **Final symmetric expansion** to absorb any new slack opened by step 3.

### Why coordinate refinement matters here

Per-battery DP plans each battery as if alone on the network. PTDF projection
then enforces feasibility but only by *shrinking* contributors to a violated
line, never by reallocating among batteries with different node-line coupling.
When two batteries have opposing PTDF on a tight line, the DP-projected action
leaves headroom that the projection cannot redistribute. Coordinate refinement,
operating against the same DP value function, fills that gap.

### What's different from the public baselines

`greedy` looks at node 0 as a proxy and bang-bangs on a fixed price gap;
`conservative` looks at the per-step cross-node average and bang-bangs on a
±5% threshold. Neither references SOC, neither plans across the horizon, and
neither uses the realised RT price. This policy uses all three: per-node DA
for planning, realised RT for execution, and a per-battery value function +
PTDF-aware refinement carry the SOC, horizon, and network coupling jointly.

## Hyperparameters

| Field | Default | Meaning |
|---|---|---|
| `horizon_steps` | 192 | Window cap for the std/jump statistics (auto-clipped to remaining steps). |
| `min_window_std` | 0.0 | Skip trading when the window price std is below this; `0.0` means never skip. |
| `jump_z_threshold` | 5.0 | Standardised-RT magnitude that triggers full-bound bang override. |
| `dp_soc_levels` | 24 | SOC discretisation count for the per-battery DP. |
| `dp_action_levels` | 21 | Action-grid resolution for backward induction, online selection, and coordinate refinement. |

Legacy fields (`urgency_gain`, `rt_z_gain`, `action_sharpness`,
`profit_floor_shrink`, `congestion_premium_scale`) are retained on the struct
for serde compatibility with prior versions but are not used by the current
policy.

Under the locked defaults the policy beats `max(greedy, conservative)` on
**50 of 50 challenge instances** (5 scenarios × 10 seeds) with aggregate
profit 10.5× baseline ($12.10M vs $1.15M). Mean per-scenario clamped quality:

| Scenario | mean quality (clamp ±10) |
|---|---|
| BASELINE | +2.73 |
| CONGESTED | +5.52 |
| MULTIDAY | +8.39 |
| DENSE | +9.43 |
| CAPSTONE | +8.97 |
| **total / max 50** | **35.04** |

This is +1.60 over the prior congestion-premium DP policy (33.44), driven
entirely by the post-projection coordinate refinement which captures the
multi-battery line-slack reallocation that simple projection-shrink misses.

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
