#!/usr/bin/env bash
# Build the numen binary with the `zombienet-runtime` feature, and eth-rpc for
# the scenarios that drive the Ethereum RPC.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

cargo build --release -p numen --features zombienet-runtime

BIN="$ROOT/target/release/numen"
echo "Built: $BIN"
"$BIN" --version

bash .maintain/build-eth-rpc.sh target/eth-rpc
target/eth-rpc/bin/eth-rpc --version

# Pre-generate the raw chainspec consumed by zombienet.toml. zombienet
# would otherwise rebuild a spec from --chain integration on every spawn
# and overwrite our pre-registered session keys with auto-generated ones.
SPEC="$ROOT/zombienet/integration/integration-raw.json"
echo "Generating raw chainspec at $SPEC"
"$BIN" build-spec --chain integration --raw --disable-default-bootnode > "$SPEC"
echo "Raw spec: $(wc -c < "$SPEC") bytes"
