# TIG Code Submission

## Submission Details

* **Challenge Name:** energy_arbitrage
* **Algorithm Name:** nodal_temporal_arb
* **Copyright:** 2026 [submitter to fill]
* **Identity of Submitter:** [submitter to fill]
* **Identity of Creator of Algorithmic Method:** [submitter to fill]
* **Unique Algorithm Identifier (UAI):** null

## Method

A myopic per-step policy with three components the public baselines lack:

1. **Per-node price-rank look-ahead.** For each battery at node `n`, the policy
   reads the next `horizon_steps` of day-ahead prices *at that node* (greedy
   uses node 0 as a proxy; conservative uses an across-nodes average at the
   current step only). It computes the rank of the current DA price within
   that window and uses `target_frac = 1 - rank` as the desired state-of-charge
   fraction. When the current price is the lowest in the window the target is
   "full"; when it is the highest, the target is "empty".

2. **SOC-aware urgency.** The action is driven by `(current_frac - target_frac)`,
   not by absolute price thresholds. A nearly-full battery at a moderately high
   price discharges; a nearly-empty battery at a moderately low price charges.
   This closes a gap in both baselines, which never reference SOC except via
   `action_bounds` clamping.

3. **Magnitude-aware RT correction.** The deviation `(rt_price[n] - p_now)` is
   standardised by the window's price standard deviation and passed through a
   `tanh`. Tail RT spikes saturate to ±1 and dominate the urgency term, so
   large jumps trigger near-bang-bang responses without any threshold tuning.

The combined signal is mapped to MW via the available bounds (negative signal
→ charge toward `lo`, positive → discharge toward `hi`), then projected onto
PTDF feasibility (greedy line softening + binary-scale fallback, copied from
the published baseline) and clipped by a profit floor that shrinks magnitudes
when cumulative profit would otherwise go negative.

## Hyperparameters

| Field | Default | Meaning |
|---|---|---|
| `horizon_steps` | 192 | Look-ahead window cap (auto-clipped to remaining steps; the policy uses all available DA-price information). |
| `urgency_gain` | 0.5 | Weight on `(current_frac − target_frac)` before the saturating non-linearity. |
| `rt_z_gain` | 1.5 | Gain on standardised RT deviation. |
| `min_window_std` | 1.0 | Skip trading when window price std (\$/MWh) is below this. |
| `action_sharpness` | 0.5 | Tanh sharpness on combined signal; lower → more bang-bang. |
| `profit_floor_shrink` | 0.95 | Per-iter shrink factor when cumulative profit floor binds. |

The defaults were selected by sweep on 25 challenge instances (5 scenarios ×
5 seeds). Under these settings the policy beats `max(greedy, conservative)`
on 22 of 25 instances; the three losses are all on the BASELINE scenario
(low volatility, loose congestion) where the smooth-action policy
slightly underperforms the published bang-bang baselines.

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
