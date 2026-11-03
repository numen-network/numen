# EVM developer-tooling compatibility

This directory verifies that the numen EVM layer, pallet-revive, works with the mainstream Ethereum toolchain.

## Layout

| Sub-directory | Tool                     | What it covers                                                  |
| ------------- | ------------------------ | --------------------------------------------------------------- |
| `hardhat/`    | Hardhat + ethers v6      | compile / deploy / live test of an ERC-20-like contract         |
| `foundry/`    | Foundry (`forge`/`cast`) | `forge build`, `forge create`, `forge script`, `cast call/send` |
| `scripts/`    | ethers v6 + web3 v4      | native coin transfer + read-only RPC smoke checks               |

The corresponding zombienet end-to-end scenario lives at [`../integration/scenarios/08-evm-tooling.zndsl`](../integration/scenarios/08-evm-tooling.zndsl) and is exercised by [`../integration/scripts/run-all.sh`](../integration/scripts/run-all.sh).

## Chain parameters

| Field              | Value                   |
| ------------------ | ----------------------- |
| RPC URL (default)  | `http://127.0.0.1:9944` |
| WebSocket URL      | `ws://127.0.0.1:9944`   |
| Chain ID           | `320262`                 |
| Currency symbol    | `NUMN`                  |
| Decimals           | `18`                    |
| Block explorer URL | _(none yet)_            |

Start a single-node dev chain that exposes both the substrate and Ethereum RPC namespaces on the same port:

```bash
./target/release/numen --dev --rpc-cors all --rpc-port 9944
```

## Pre-funded development accounts

Every preset (`development`, `local_testnet`, `integration`) pre-funds the six well-known **Ethereum dev accounts** with `1,000,000 NUMN` each. Their private keys are publicly documented and ship with common EVM dev tooling; **they MUST NOT be used in any non-development network.**

| Name      | Address                                      | Private key                                                          |
| --------- | -------------------------------------------- | -------------------------------------------------------------------- |
| Alith     | `0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac` | `0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133` |
| Baltathar | `0x3Cd0A705a2DC65e5b1E1205896BaA2be8A07c6e0` | `0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b` |
| Charleth  | `0x798d4Ba9baf0064Ec19eB4F0a1a45785ae9D6DFc` | `0x0b6e18cafb6ed99687ec547bd28139cafdd2bffe70e6b688025de6b445aa5c5b` |
| Dorothy   | `0x773539d4Ac0e786233D90A233654ccEE26a613D9` | `0x39539ab1876910bbf3a223d84a29e28f1cb4e2e456503e7e91ed39b2e7223d68` |
| Ethan     | `0xFf64d3F6efE2317EE2807d223a0Bdc4c0c49dfDB` | `0x7dce9bc8babb68fec1409be38c8e1a52650206a7ed90ff956ae8a6d15eeaaef4` |
| Faith     | `0xC0F0f4ab324C46e55D02D0033343B4Be8A55532d` | `0xb9d2ea9a615f3165812e8d44de0d24da9bbd164b65c4f0573e1ce2c8dbd9c8df` |

## MetaMask walkthrough

MetaMask is exercised manually; the steps below are the canonical recipe.

### 1. Add the network

Open MetaMask → **Networks → Add a custom network**:

- Network name: `Numen Dev`
- New RPC URL: `http://127.0.0.1:9944`
- Chain ID: `320262`
- Currency symbol: `NUMN`
- Block explorer URL: _(leave blank)_

Save. MetaMask will validate that the RPC responds to `eth_chainId` and returns `0x4e306` (== 320262).

### 2. Import a pre-funded account

**Account menu → Add account or hardware wallet → Import account → Private key**, paste any of the keys from the table above (Alith is the canonical choice). The balance should immediately render as `1,000,000 NUMN`.

### 3. Native coin transfer

From the imported account, send any amount to a second imported account (e.g. Baltathar). Once the dev node mines the inclusion block (~20 s default `TargetBlockTime`), MetaMask reports the tx as **Confirmed** and both balances update accordingly.

The same flow is automated by [`scripts/transfer-ethers.js`](scripts/transfer-ethers.js).

### 4. Contract interaction

1. Deploy `Token.sol` with Hardhat:

   ```bash
   cd zombienet/evm-tooling/hardhat
   npm install
   npx hardhat run scripts/deploy.js --network numen
   # prints: Token: 0x...
   ```

2. In MetaMask: **Tokens → Import tokens → Custom token**, paste the printed address. Symbol auto-fills as `CNT`, decimals as `18`.

3. Use **Send** to transfer `CNT` between imported accounts. MetaMask constructs the `transfer(address,uint256)` calldata, signs with the imported key, and submits to the Frontier RPC. The receipt and updated balances appear once the next block is mined.

If any step fails, capture the MetaMask "Activity → Speed up / Cancel" detail panel and the dev-node log; both are required for a useful bug report.

## Native NUMN as ERC20 (precompile `0x0802`)

The runtime exposes the native balance pallet through an ERC20-shaped precompile at `0x0000000000000000000000000000000000000802` (`pallet-evm-precompile-balances-erc20`). It serves two purposes:

1. **EVM-native NUMN as ERC20** — `name` / `symbol` / `decimals` / `totalSupply` / `balanceOf` / `transfer` / `transferFrom` / `approve` / `allowance` are routed straight to `pallet-balances`. Wallets and contracts can therefore handle native NUMN through the same ERC20 tooling they already use.
2. **EVM → Substrate bridge** — the non-standard `withdraw(bytes32 dest, uint256 amount)` selector moves `amount` from the caller's mirror substrate account to the substrate `AccountId32` given by `dest`, emitting `Withdrawal(address,bytes32,uint256)`. This is the supported way to move funds **back from the EVM side to a pure substrate account** (e.g. Alice / Bob / sudo) — a vanilla EVM `value` transfer cannot reach an `AccountId32` that has no `H160` preimage.

The Rust end-to-end coverage lives in [`runtime/tests/evm.rs`](../../runtime/tests/evm.rs) (`balances_erc20_precompile_balance_of_and_transfer_via_runner`). The JavaScript counterparts that drive the precompile through the live JSON-RPC are:

| Path                                                                                                             | What it covers                                                 |
| ---------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| [`scripts/transfer-precompile.js`](scripts/transfer-precompile.js)                                               | ERC20 `transfer` of native NUMN (Alith → Baltathar via 0x0802) |
| [`scripts/withdraw-to-substrate.js`](scripts/withdraw-to-substrate.js)                                           | `withdraw(bytes32,uint256)` — Alith (EVM) → Alice (substrate)  |
| [`hardhat/test/NativeErc20.test.js`](hardhat/test/NativeErc20.test.js)                                           | Hardhat assertion suite for `balanceOf` / `transfer` at 0x0802 |
| [`../integration/js-scripts/evm-precompile-roundtrip.js`](../integration/js-scripts/evm-precompile-roundtrip.js) | Zombienet round-trip: substrate → EVM → substrate              |

### MetaMask: import native NUMN as a token

In MetaMask: **Tokens → Import tokens → Custom token**, paste `0x0000000000000000000000000000000000000802`. Symbol auto-fills as `NUMN`, decimals as `18`. The reported balance now mirrors the same account's `eth_getBalance`, and `Send` constructs an ERC20 `transfer(address,uint256)` against the precompile instead of a value transaction. Withdrawing to a substrate account requires constructing the `withdraw` calldata manually (e.g. through Etherscan-style "interact with contract" dialogs); the canonical example is in `scripts/withdraw-to-substrate.js`.
