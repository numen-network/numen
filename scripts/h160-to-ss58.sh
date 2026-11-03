#!/usr/bin/env bash
# h160-to-ss58.sh
#
# Convert an Ethereum H160 address to the Substrate AccountId32 / SS58 address
# pallet-revive's `AccountId32Mapper` resolves it to, and to the account
# Frontier kept its funds at.
#
# Mapping rules:
#     account32 = h160_bytes ++ 0xEE * 12
#     frontier  = blake2_256("evm:" ++ h160_bytes)
#
# An H160 that belongs to a substrate account resolves to that account
# instead, which only the chain knows. Funds still at the Frontier account
# move over with `EvmClaims.claim`.
#
# Default SS58 prefix: 14240 (this runtime's `SS58Prefix`).
#
# Usage:
#     scripts/h160-to-ss58.sh <0xH160> [ss58_prefix]
#
# Examples:
#     scripts/h160-to-ss58.sh 0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac
#     scripts/h160-to-ss58.sh 0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac 42

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "Usage: $0 <0xH160> [ss58_prefix]" >&2
    exit 1
fi

H160_RAW="${1#0x}"
SS58_PREFIX="${2:-14240}"

if [[ ! "$H160_RAW" =~ ^[0-9a-fA-F]{40}$ ]]; then
    echo "Error: H160 must be 40 hex chars (with optional 0x prefix)" >&2
    exit 1
fi

PUBKEY_HEX="${H160_RAW,,}eeeeeeeeeeeeeeeeeeeeeeee"
FRONTIER_HEX=$(python3 - "$H160_RAW" <<'PY'
import sys, hashlib
h160 = bytes.fromhex(sys.argv[1])
print(hashlib.blake2b(b"evm:" + h160, digest_size=32).hexdigest())
PY
)

ss58() {
    if command -v subkey >/dev/null 2>&1; then
        local address
        address=$(subkey inspect --network "${SS58_PREFIX}" "0x$1" 2>/dev/null \
            | awk -F': *' '/SS58 Address/ {print $2; exit}')
        if [[ -n "${address:-}" ]]; then
            echo "$address"
            return
        fi
    fi

    # Fallback: derive SS58 in pure Python (no subkey required).
    python3 - "$1" "$SS58_PREFIX" <<'PY'
import sys, hashlib

PUBKEY = bytes.fromhex(sys.argv[1])
PREFIX = int(sys.argv[2])

# SS58 prefix encoding (single or two-byte form).
if PREFIX < 64:
    prefix_bytes = bytes([PREFIX])
elif PREFIX < 16384:
    lo = ((PREFIX & 0b1111_1100) >> 2) | 0b0100_0000
    hi = (PREFIX >> 8) | ((PREFIX & 0b0000_0011) << 6)
    prefix_bytes = bytes([lo, hi])
else:
    raise SystemExit("ss58 prefix out of range")

payload = prefix_bytes + PUBKEY
checksum = hashlib.blake2b(b"SS58PRE" + payload, digest_size=64).digest()[:2]
raw = payload + checksum

ALPHABET = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
n = int.from_bytes(raw, "big")
out = bytearray()
while n > 0:
    n, r = divmod(n, 58)
    out.append(ALPHABET[r])
for b in raw:
    if b == 0:
        out.append(ALPHABET[0])
    else:
        break
print(out[::-1].decode())
PY
}

printf '%-26s%s\n' \
    "H160:" "0x${H160_RAW,,}" \
    "AccountId32 (hex):" "0x${PUBKEY_HEX}" \
    "SS58 (prefix=${SS58_PREFIX}):" "$(ss58 "$PUBKEY_HEX")" \
    "Frontier account (hex):" "0x${FRONTIER_HEX}" \
    "Frontier SS58:" "$(ss58 "$FRONTIER_HEX")"
