// 008 EVM developer-tooling smoke test, driven via a single zombienet js-script.
//
// Verifies, over ethers v6 + Frontier JSON-RPC, the whole EVM tooling surface
// this runtime owns:
//   1. config smoke + cross-node consistency: eth_chainId == 320262 and the
//      initial baseFeePerGas == 1000 gwei, identical when read from alice/bob/
//      charlie (validates pallet-evm ChainId and pallet-base-fee defaults).
//   2. minimal deploy path: deploy init code that returns a single STOP byte,
//      then read the runtime code back and assert it equals 0x00.
//   3. SS58 -> EVM: Alice tops up Alith's HashedAddressMapping account with
//      10 UNIT via balances.transferKeepAlive; Alith's EVM balance grows 10 UNIT.
//   4. EVM -> SS58 (type-2): Alith calls withdraw(dest, 5 UNIT) on 0x0802 with
//      an EIP-1559 type-2 transaction; receipt is type 2 and status 1.
//   5. both bridges reconcile exactly on both sides (fee from TransactionFeePaid
//      on substrate, from gasUsed * gasPrice on EVM).
//
// Returns 1 on success, 0 on failure (zombienet `js-script ... return is 1`).
//
// Usage: js-script ./evm-tooling.js

const {
    WebSocketProvider, Wallet, Contract,
    keccak256, toUtf8Bytes, getAddress,
} = require("ethers");
const { blake2AsU8a } = require("@polkadot/util-crypto");
const { hexToU8a, stringToU8a, u8aConcat, u8aToHex } = require("@polkadot/util");
const { connect, disconnectAll, pair, keyring, submitExtrinsic } = require("../../js-scripts/lib");

// Frontier dev account (Alith), pre-funded by the integration genesis preset.
const ALITH_PK =
    "0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133";
const PRECOMPILE = "0x0000000000000000000000000000000000000802";
const ABI = ["function withdraw(bytes32,uint256) returns (bool)"];

const EXPECTED_CHAIN_ID = 320262n;
const EXPECTED_BASE_FEE = 1_000_000_000_000n; // DefaultBaseFeePerGas, 1000 gwei.
const UNIT = 10n ** 18n; // runtime UNIT == 1e18, EVM uses 18 decimals.
const TOPUP = 10n * UNIT; // SS58 -> EVM amount.
const WITHDRAW = 5n * UNIT; // EVM -> SS58 amount.

// Init bytecode that returns a 1-byte runtime: STOP (0x00).
const INIT_CODE = "0x6001600c60003960016000f300";
const EXPECTED_RUNTIME = "0x00";

// Frontier HashedAddressMapping<BlakeTwo256>: AccountId = blake2_256("evm:" ++ h160).
function substrateAddressFromH160(k, h160) {
    const data = u8aConcat(stringToU8a("evm:"), hexToU8a(h160));
    return k.encodeAddress(blake2AsU8a(data, 256));
}

async function run(_zombie, networkInfo, _args) {
    const evmProviders = {};
    let api;
    try {
        // --- Step 1: config smoke + cross-node consistency. ---
        let chainId = null;
        let baseFee = null;
        for (const name of ["alice", "bob", "charlie"]) {
            const info = networkInfo.nodesByName[name];
            if (!info) {
                console.error("📜", `  node ${name} not in networkInfo`);
                return 0;
            }
            const provider = new WebSocketProvider(info.wsUri);
            evmProviders[name] = provider;

            const id = (await provider.getNetwork()).chainId;
            // Read the base fee at the genesis block #0: that is the untouched
            // DefaultBaseFeePerGas before pallet-base-fee starts decaying it on
            // empty blocks, and #0 is canonical so every node reports the same.
            const block = await provider.getBlock(0);
            const fee = block.baseFeePerGas;
            console.log("📜", `  ${name}: chainId=${id} baseFeePerGas(#0)=${fee}`);

            if (id !== EXPECTED_CHAIN_ID) {
                console.error("📜", `  ${name} chainId ${id} != ${EXPECTED_CHAIN_ID}`);
                return 0;
            }
            if (fee !== EXPECTED_BASE_FEE) {
                console.error("📜", `  ${name} baseFeePerGas ${fee} != ${EXPECTED_BASE_FEE}`);
                return 0;
            }
            if (chainId === null) {
                chainId = id;
                baseFee = fee;
            } else if (id !== chainId || fee !== baseFee) {
                console.error("📜", `  ${name} disagrees: chainId=${id} baseFee=${fee}`);
                return 0;
            }
        }
        console.log("📜", "  config smoke ok: chainId/baseFee consistent across nodes");

        const provider = evmProviders["alice"];
        const wallet = new Wallet(ALITH_PK, provider);

        // --- Step 2: minimal deploy path. ---
        const deployTx = await wallet.sendTransaction({ data: INIT_CODE, gasLimit: 200_000n });
        const deployReceipt = await deployTx.wait();
        if (!deployReceipt || deployReceipt.status !== 1) {
            console.error("📜", `  deploy failed: status=${deployReceipt && deployReceipt.status}`);
            return 0;
        }
        const addr = deployReceipt.contractAddress;
        const code = await provider.getCode(addr);
        if (code.toLowerCase() !== EXPECTED_RUNTIME) {
            console.error("📜", `  runtime code ${code} != ${EXPECTED_RUNTIME}`);
            return 0;
        }
        console.log("📜", `  deploy ok: ${addr} runtime code ${code}`);

        // --- Step 3: SS58 -> EVM top-up. ---
        // Use Ferdie as the sender: it is pre-funded but neither validates nor
        // mines, so its balance is not perturbed by block rewards while we
        // reconcile the transfer (alice/bob/charlie/dave all mine).
        api = await connect(networkInfo, "alice");
        const k = await keyring();
        const sender = await pair("//Ferdie");
        const sink = await pair("//EvmWithdrawSink");
        const alithSubstrate = substrateAddressFromH160(k, wallet.address);

        const alithEvmBefore = await provider.getBalance(wallet.address);
        const senderSubBefore = (await api.query.system.account(sender.address)).data.free.toBigInt();

        const { events } = await submitExtrinsic(
            api, sender,
            api.tx.balances.transferKeepAlive(alithSubstrate, TOPUP.toString()),
        );

        const alithEvmAfterTopup = await provider.getBalance(wallet.address);
        const senderSubAfter = (await api.query.system.account(sender.address)).data.free.toBigInt();

        const evmGain = alithEvmAfterTopup - alithEvmBefore;
        if (evmGain !== TOPUP) {
            console.error("📜", `  Alith EVM gain ${evmGain} != ${TOPUP}`);
            return 0;
        }
        const feePaid = events.find(({ event }) =>
            api.events.transactionPayment.TransactionFeePaid.is(event));
        if (!feePaid) {
            console.error("📜", "  transfer emitted no TransactionFeePaid");
            return 0;
        }
        const txFee = feePaid.event.data.actualFee.toBigInt();
        const senderSpent = senderSubBefore - senderSubAfter;
        if (senderSpent !== TOPUP + txFee) {
            console.error("📜", `  Ferdie spent ${senderSpent} != ${TOPUP} + fee ${txFee}`);
            return 0;
        }
        console.log("📜", `  SS58->EVM ok: Alith +${evmGain}, Ferdie -${senderSpent} (fee ${txFee})`);

        // --- Step 4: EVM -> SS58 withdraw via 0x0802 (EIP-1559 type-2). ---
        const erc20 = new Contract(PRECOMPILE, ABI, wallet);
        const destBytes32 = u8aToHex(sink.publicKey);
        const sinkSubBefore = (await api.query.system.account(sink.address)).data.free.toBigInt();
        const alithEvmBeforeWithdraw = await provider.getBalance(wallet.address);

        const wTx = await erc20.withdraw(destBytes32, WITHDRAW, {
            type: 2,
            gasLimit: 200_000n,
        });
        const wReceipt = await wTx.wait();
        if (!wReceipt || wReceipt.status !== 1 || wReceipt.type !== 2) {
            console.error("📜", `  withdraw bad receipt: status=${wReceipt && wReceipt.status} type=${wReceipt && wReceipt.type}`);
            return 0;
        }

        // --- Step 5: reconcile both sides of the withdraw. ---
        const sinkSubAfter = (await api.query.system.account(sink.address)).data.free.toBigInt();
        const alithEvmAfterWithdraw = await provider.getBalance(wallet.address);

        const sinkGain = sinkSubAfter - sinkSubBefore;
        if (sinkGain !== WITHDRAW) {
            console.error("📜", `  sink gain ${sinkGain} != ${WITHDRAW}`);
            return 0;
        }
        const gasFee = wReceipt.gasUsed * wReceipt.gasPrice;
        const alithDrop = alithEvmBeforeWithdraw - alithEvmAfterWithdraw;
        if (alithDrop !== WITHDRAW + gasFee) {
            console.error("📜", `  Alith drop ${alithDrop} != ${WITHDRAW} + gasFee ${gasFee}`);
            return 0;
        }
        console.log("📜", `  EVM->SS58 ok (type-2): sink +${sinkGain}, Alith -${alithDrop} (gas ${gasFee})`);

        console.log("📜", "  all EVM tooling checks passed");
        return 1;
    } catch (e) {
        console.error("📜", `  ${e.stack || e.message}`);
        return 0;
    } finally {
        await disconnectAll(api ? [api] : []);
        for (const p of Object.values(evmProviders)) {
            try { await p.destroy(); } catch (_) {}
        }
    }
}

module.exports = { run };
