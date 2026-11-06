# Hardhat compatibility sample

End-to-end Hardhat 3 workflow against a local numen dev chain. Verifies that compile → deploy → call → state-readback work over the Ethereum JSON-RPC the node serves through eth-rpc.

## Prerequisites

- Node.js >= 22 (tested with v24)
- A locally built node (release recommended for the integration round-trip):

  ```bash
  cargo build --release
  ```

## Run

```bash
# 1. start a single-node dev chain in another terminal
./target/release/numen --dev --rpc-cors all --rpc-port 9944

# 2. install JS deps
cd zombienet/evm-tooling/hardhat
npm install

# 3. compile the contract
npx hardhat compile

# 4. run the live-chain test suite
npx hardhat --network numen test mocha

# 5. (optional) deploy without running tests
npx hardhat --network numen run scripts/deploy.js
```

Override the RPC endpoint with `NUMEN_RPC=http://host:port npx hardhat ...`.

## Pre-funded accounts

The dev / local_testnet / integration genesis presets pre-fund the well-known Ethereum dev accounts. The first three are wired into `hardhat.config.js`:

| Name      | Address                                      |
| --------- | -------------------------------------------- |
| Alith     | `0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac` |
| Baltathar | `0x3Cd0A705a2DC65e5b1E1205896BaA2be8A07c6e0` |
| Charleth  | `0x798d4Ba9baf0064Ec19eB4F0a1a45785ae9D6DFc` |

Their private keys are publicly documented and MUST NOT be used in any non-development network.
