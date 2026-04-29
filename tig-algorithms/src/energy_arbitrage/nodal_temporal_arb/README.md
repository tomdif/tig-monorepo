# TIG Code Submission

## Submission Details

* **Challenge Name:** energy_arbitrage
* **Algorithm Name:** nodal_temporal_arb
* **Copyright:** 2026 [submitter to fill]
* **Identity of Submitter:** [submitter to fill]
* **Identity of Creator of Algorithmic Method:** [submitter to fill]
* **Unique Algorithm Identifier (UAI):** null

## Method

A myopic per-step policy with four components the public baselines lack:

1. **Per-node price-rank look-ahead.** For each battery at node `n`, the policy
   reads the remaining day-ahead prices *at that node* (greedy uses node 0 as
   a proxy; conservative uses an across-nodes average at the current step
   only). It computes the rank of the current DA price within that window
   and uses `target_frac = 1 - rank` as the desired state-of-charge fraction.
   When the current price is the lowest in the window the target is "full";
   when it is the highest, the target is "empty".

2. **SOC-aware urgency.** The action is driven by `(current_frac - target_frac)`,
   not by absolute price thresholds. A nearly-full battery at a moderately high
   price discharges; a nearly-empty battery at a moderately low price charges.
   This closes a gap in both baselines, which never reference SOC except via
   `action_bounds` clamping.

3. **Magnitude-aware RT correction.** The deviation `(rt_price[n] - p_now)` is
   standardised by the window's price standard deviation and passed through a
   `tanh`. Moderate RT divergences nudge the smoothed signal in the deviation
   direction.

4. **Tail-jump bang override.** When the standardised RT deviation exceeds
   `jump_z_threshold` window-stds, the smoothed pipeline is bypassed and a
   full-bound action in the direction of the jump is committed. This captures
   Pareto jumps in the RT price model (`α_tail` as low as 2.5 in CAPSTONE)
   without relying on the smooth tanh to saturate.

The combined signal is mapped to MW via the available bounds (negative signal
→ charge toward `lo`, positive → discharge toward `hi`), then projected onto
PTDF feasibility (greedy line softening + binary-scale fallback, copied from
the published baseline) and clipped by a profit floor that shrinks magnitudes
when cumulative profit would otherwise go negative.

## Hyperparameters

| Field | Default | Meaning |
|---|---|---|
| `horizon_steps` | 192 | Look-ahead window cap (auto-clipped to remaining steps; the policy uses all available DA-price information). |
| `urgency_gain` | 1.0 | Weight on `(current_frac − target_frac)` before the saturating non-linearity. |
| `rt_z_gain` | 1.0 | Gain on standardised RT deviation in the smoothed branch. |
| `min_window_std` | 0.0 | Skip trading when window price std (\$/MWh) is below this; `0.0` means never skip. |
| `action_sharpness` | 0.30 | Tanh sharpness on combined signal; lower → more bang-bang. |
| `profit_floor_shrink` | 0.95 | Per-iter shrink factor when cumulative profit floor binds. |
| `jump_z_threshold` | 2.0 | Standardised-RT magnitude that triggers full-bound bang override; set to 0 to disable. |

The defaults were selected by sweep on 50 challenge instances (5 scenarios ×
10 seeds), scoring with TIG's per-nonce quality formula
`((profit − baseline) / |baseline|).clamp(-10, 10)`. Under these settings the
policy beats `max(greedy, conservative)` on **45 of 50 instances** (8.6×
aggregate profit) with mean per-scenario clamped quality of:

| Scenario | mean quality (clamp ±10) |
|---|---|
| BASELINE | +2.24 |
| CONGESTED | +4.32 |
| MULTIDAY | +8.04 |
| DENSE | +9.04 |
| CAPSTONE | +8.58 |
| **total / max 50** | **32.22** |

Losses concentrate on BASELINE (low volatility, loose congestion), where the
smooth-action policy slightly underperforms the bang-bang baselines on a
handful of seeds. In aggregate the BASELINE scenario is still net positive
because the rank-driven targeting captures large wins on the seeds where the
baselines happen to do poorly.

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
