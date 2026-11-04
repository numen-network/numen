# ethers / web3 sample scripts

Stand-alone Node.js scripts that exercise the node's Ethereum JSON-RPC through both `ethers` v6 and `web3` v4.

## Run

```bash
# 1. start a local dev chain
./target/release/numen --dev --rpc-cors all --rpc-port 9944

# 2. install JS deps
cd zombienet/evm-tooling/scripts
npm install

# 3. native coin transfer (Alith → Baltathar)
node transfer-ethers.js

# 4. read-only RPC smoke check
node query-web3.js

# 5. native NUMN as ERC20 via the 0x0802 precompile (Alith → Baltathar)
node transfer-precompile.js

# 6. EVM → substrate withdrawal via 0x0802 (Alith → Alice)
node withdraw-to-substrate.js
```

Override the endpoint with `NUMEN_RPC=http://host:port`.

Each script exits non-zero on assertion failure:

- `transfer-ethers.js` — recipient balance delta must equal the transfer amount.
- `query-web3.js` — chain id must be `320262`.
- `transfer-precompile.js` — `balanceOf` must equal `eth_getBalance` and the
  recipient delta must match the transfer amount.
- `withdraw-to-substrate.js` — the receipt must contain a
  `Withdrawal(address,bytes32,uint256)` log whose fields match the call,
  and the caller's balance must drop by at least the requested amount.
