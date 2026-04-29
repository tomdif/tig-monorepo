# TIG Code Submission

## Submission Details

* **Challenge Name:** energy_arbitrage
* **Algorithm Name:** nodal_temporal_arb
* **Copyright:** 2026 [submitter to fill]
* **Identity of Submitter:** [submitter to fill]
* **Identity of Creator of Algorithmic Method:** [submitter to fill]
* **Unique Algorithm Identifier (UAI):** null

## Method

A myopic per-step policy that combines two arbitrage signals the existing
greedy baseline does not:

1. **Per-node temporal forecast.** For each battery at node `n`, the policy
   reads the next `horizon_steps` of day-ahead prices *at that node* (the
   reference baseline reads only node 0). It normalizes the current node DA
   price against the rolling [min, max] range and converts that percentile
   into a continuous action in `[-power_charge_mw, +power_discharge_mw]`,
   with a deadband around mid-range and a minimum-spread floor that prevents
   trading when the forecast spread is below the round-trip efficiency loss.

2. **Real-time correction.** The deviation of the current real-time price
   from the DA forecast at the same node is blended into the action signal
   with weight `rt_blend`, so transient RT spikes nudge the policy toward
   discharge (or absorb troughs by charging harder).

3. **Feasibility projection.** Greedy line-by-line softening of the most
   violated PTDF flow, followed by binary-search global scaling — same
   projection scheme as the published baseline, retained because it is
   already correct and cheap.

## Hyperparameters

| Field | Default | Meaning |
|---|---|---|
| `horizon_steps` | 24 | Look-ahead window in 15-min steps (6 h). |
| `charge_threshold` | 0.30 | Charge when normalized DA percentile is below this. |
| `discharge_threshold` | 0.70 | Discharge when normalized DA percentile is above this. |
| `min_spread` | 5.0 | Skip trading when DA window spread (\$/MWh) is below this. |
| `rt_blend` | 0.20 | Weight of RT correction relative to DA signal. |
| `rt_scale` | 50.0 | Normalizer for RT deviation (\$/MWh). |

## References and Acknowledgments

### Code References

* Feasibility projection (greedy line softening + binary-scale fallback)
  adapted from the published `energy_arbitrage` greedy baseline in
  `tig-challenges/src/energy_arbitrage/baselines/greedy.rs`.

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
