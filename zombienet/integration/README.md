# Multi-Node Integration & Zombienet Tests

End-to-end test harness for the numen multi-node network. Drives a 5-node topology with [zombienet](https://github.com/paritytech/zombienet) and exercises PoW production, GRANDPA finality, validator lifecycle (lock/exit), difficulty adjustment, and network-partition recovery.

## Runtime parameters (`zombienet-runtime` feature)

The node binary used by this harness MUST be compiled with `--features zombienet-runtime`. Otherwise session/lock/cooldown/difficulty timing cannot be observed within scenario timeouts.

With `TargetBlockTime = 20s` (so `MINUTES = 3` blocks) the derived constants are:

| Constant                         | zombienet-runtime |
| -------------------------------- | -----------: |
| `SessionPeriod`                  |       3 mins |
| `LockAmount`                     |       1 NUMN |
| `LockDuration`                   |       5 mins |
| `RenewInterval`                  |       3 mins |
| `RejoinCooldownPeriod`           |       1 mins |
| `OfflineThreshold`               |            1 |
| `MaxValidators`                  |            4 |
| `DifficultyHalflife`             |          60s |
| `DifficultyBreakThresholdSecs`   |         120s |

## Prerequisites

```bash
# Node.js (>= 18; tested with v24) and npm
apt install curl unzip
curl -o- https://fnm.vercel.app/install | bash
fnm install 24
node -v # Should print "v24.15.0" or newer.
npm -v # Should print "11.12.1" or newer.

# zombienet binary (linux x86_64 example, tested with v1.3.138)
sudo curl -L -o /usr/local/bin/zombienet \
  https://github.com/paritytech/zombienet/releases/download/v1.3.138/zombienet-linux-x64
sudo chmod +x /usr/local/bin/zombienet
zombienet version
```

## Quick start

```bash
cd zombienet/integration

# 1. install JS deps (used by js-script blocks)
npm install

# 2. build node with zombienet-runtime and eth-rpc, AND pre-generate the raw
#    chainspec. Re-run after every change to runtime/ or the integration preset.
bash scripts/build-node.sh

# 3. run all scenarios (auto-creates /tmp/zn-creds.cfg if missing)
bash scripts/run-all.sh
# or individually (note `-p native` is implied by zombienet.toml):
zombienet -p native test scenarios/.../zombienet.zndsl
```
