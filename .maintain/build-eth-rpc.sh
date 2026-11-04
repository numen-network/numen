#!/usr/bin/env bash
# Installs eth-rpc under <root>/bin, built from the same polkadot-sdk commit as
# the runtime.

set -euo pipefail

if (($# == 0)); then
	echo "install root required, e.g. $0 target/eth-rpc [cargo install options]" >&2
	exit 1
fi
root=$1
shift

lock="$(dirname "${BASH_SOURCE[0]}")/../Cargo.lock"
rev=$(sed -nE 's|^source = "git\+https://github\.com/paritytech/polkadot-sdk\?[^#]*#([0-9a-f]{40})"$|\1|p' "$lock" | sort -u)
if (($(wc -w <<<"$rev") != 1)); then
	echo "Cargo.lock should pin polkadot-sdk at exactly one commit" >&2
	echo "$rev" >&2
	exit 1
fi

# eth-rpc only reads metadata from the dev runtime's native build, so skip the
# wasm build.
SKIP_WASM_BUILD=1 cargo install pallet-revive-eth-rpc --locked \
	--git https://github.com/paritytech/polkadot-sdk --rev "$rev" --root "$root" "$@"
