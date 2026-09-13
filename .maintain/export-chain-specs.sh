#!/usr/bin/env bash
#
# Export the raw chain spec for each chain id given, e.g.
# `.maintain/export-chain-specs.sh testnet mainnet`. Genesis state comes
# from a fresh node build. Genesis `:code` is the srtool wasm published
# for the current spec_version, so a node starts on the reproducible
# runtime and the blob checks out against the release digest.
#
# A raw spec is the genesis. Export a chain only for a launch or a reset.
# Once a chain is live, leave its file alone except for bootNodes.
set -euo pipefail

cd "$(dirname "$0")/.."

if (($# == 0)); then
	echo "chain ids required, e.g. $0 testnet mainnet" >&2
	exit 1
fi

spec=$(sed -n 's/^[[:space:]]*spec_version:[[:space:]]*\([0-9_]*\),.*/\1/p' runtime/src/lib.rs | tr -d _)
tag=runtime-v$spec

# Genesis state comes from the local runtime, `:code` from the tag's
# srtool build. Both must come from the same source.
if ! git diff --quiet "$tag" -- runtime pallets consensus precompiles vendor Cargo.lock; then
	echo "runtime source differs from $tag, release the runtime first" >&2
	exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

wasm=numen_runtime-v$spec.compact.compressed.wasm
digest=numen_runtime-v$spec.srtool-digest.json
gh release download "$tag" --dir "$tmp" --pattern "$wasm" --pattern "$digest"

released=$(jq -r .runtimes.compressed.subwasm.core_version.specVersion "$tmp/$digest")
if [[ $released != "$spec" ]]; then
	echo "$tag ships spec_version $released, source says $spec" >&2
	exit 1
fi

expected=$(jq -r .runtimes.compressed.sha256 "$tmp/$digest")
actual=0x$(sha256sum "$tmp/$wasm" | cut -d' ' -f1)
if [[ $actual != "$expected" ]]; then
	echo "$wasm sha256 $actual, digest says $expected" >&2
	exit 1
fi

{ printf 0x; od -An -v -tx1 "$tmp/$wasm" | tr -d ' \n'; } > "$tmp/code.hex"

cargo build --release --locked -p numen

for chain in "$@"; do
	out=$chain-raw.json
	bootnodes='[]'
	if [[ -f $out ]]; then
		bootnodes=$(jq -c .bootNodes "$out")
	fi
	./target/release/numen build-spec --chain "$chain" --raw --disable-default-bootnode > "$tmp/$chain.json"
	jq -j --argjson bootNodes "$bootnodes" --rawfile code "$tmp/code.hex" \
		'.bootNodes = $bootNodes | .genesis.raw.top["0x3a636f6465"] = $code' \
		"$tmp/$chain.json" > "$out"
	echo "$out written with $tag and $(jq '.bootNodes | length' "$out") bootnodes"
done
