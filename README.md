# tig-monorepo (private workspace)

Private overlay for TIG algorithm submissions. **This repo contains only the
submission files**, not the full TIG monorepo.

## Layout

```
tig-algorithms/src/energy_arbitrage/nodal_temporal_arb/
  mod.rs       # the policy
  README.md    # TIG submission template (challenge name, license, etc.)
```

## How to use this on a workstation

```bash
# 1. Clone upstream TIG monorepo (this is the build/test environment)
git clone https://github.com/tig-foundation/tig-monorepo.git
cd tig-monorepo

# 2. Pull this private repo's submission into the upstream tree
git clone https://github.com/tomdif/tig-monorepo.git ../tig-private
cp -r ../tig-private/tig-algorithms/src/energy_arbitrage/nodal_temporal_arb \
      tig-algorithms/src/energy_arbitrage/

# 3. Test the algorithm in Docker
docker run -it -v "$(pwd)":/app \
  ghcr.io/tig-foundation/tig-monorepo/energy_arbitrage/dev:latest

# inside the container:
build_algorithm nodal_temporal_arb
test_algorithm nodal_temporal_arb BASELINE
test_algorithm nodal_temporal_arb CONGESTED
test_algorithm nodal_temporal_arb MULTIDAY
test_algorithm nodal_temporal_arb DENSE
test_algorithm nodal_temporal_arb CAPSTONE
```

## Submission target

- Testnet first: https://test.tig.foundation (faucet TIG via https://tigstats.com/faucet on Base Sepolia)
- Mainnet: https://play.tig.foundation (10 TIG fee, finality — no edits after submit)
