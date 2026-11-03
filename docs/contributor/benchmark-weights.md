# Benchmark Weights

> How to regenerate runtime weights on the reference machine.

Weights are measured on one fixed reference machine, never on developer hardware. The reference machine defines the minimum hardware the network expects from a node. A node several times slower cannot execute full blocks inside the block interval. Swapping in a stronger reference machine therefore raises the published hardware requirement and demands a full re-run.

## One build, one lockfile

The node binary doubles as the bench host through its `benchmark pallet` subcommand. Binary and runtime wasm must come from the same `cargo build` so both sides of the benchmarking FFI share the locked SDK rev. A separately installed `frame-omni-bencher` can silently drift from the lockfile, which breaks the FFI at best and skews the numbers at worst.

## On the dev machine

One command produces both artefacts. The production profile matters since release binaries ship with it and weights must measure that same wasm.

The pallet-revive benchmarks run contract fixtures compiled during the build, so `solc` and `resolc` have to be on `PATH`. CI skips the fixtures with `SKIP_PALLET_REVIVE_FIXTURES=1`, which a weight build must not set.

```bash
cargo build --profile production --locked -p numen --features runtime-benchmarks
```

```bash
scp target/production/numen target/production/wbuild/numen-runtime/numen_runtime.compact.compressed.wasm bench:~/
```

## On the bench machine

Check out the same commit the artefacts were built from, keep the machine otherwise idle, then run the sweep. Pass pallet names to narrow it. `STEPS` and `REPEAT` override the 50 and 20 defaults.

```bash
BINARY=~/numen RUNTIME=~/numen_runtime.compact.compressed.wasm .maintain/run-benchmarks.sh
```

The script prints a `failed:` summary at the end. Weight files land in `pallets/*/src/weights.rs` and `runtime/src/weights/`.

## Bringing the numbers back

```bash
tar czf ~/weights.tgz pallets/*/src/weights.rs runtime/src/weights
```

Unpack the archive in the repo root on the dev machine and review the diff. Every file header records the host and CPU that produced it. Reject any file that does not name the reference machine.

## Excluded pallets

Some upstream benchmarks hardcode assumptions this chain rejects, such as a Root origin the governance does not grant or spend amounts far below the existential deposit. Those pallets keep their upstream weights. The list and the reason for each entry live in `runtime/src/benchmarks.rs`.
