#!/usr/bin/env bash

set -euo pipefail

if (($# == 0)); then
	echo "chain ids required, e.g. $0 testnet mainnet" >&2
	exit 1
fi

cargo build --release --locked -p numen --features metadata-hash

for chain in "$@"; do
	out=$chain-raw.json
	bootnodes='[]'
	if [[ -f $out ]]; then
		bootnodes=$(jq -c .bootNodes "$out")
	fi
	./target/release/numen build-spec --chain "$chain" --raw --disable-default-bootnode \
		| jq -j --argjson bootNodes "$bootnodes" '.bootNodes = $bootNodes' > "$out"
	echo "$out written with $(jq '.bootNodes | length' "$out") bootnodes"
done