// EVM developer-tooling smoke test, driven via a single zombienet js-script.
//
// Starts eth-rpc against alice and drives ethers v6 through alice's own RPC
// port, which hands the Ethereum namespaces to eth-rpc the way a public node
// does.
//   1. eth_chainId is 320262. eth_gasPrice is the fee multiplier at eth-rpc's
//      head, floored, times the 360M gas scale.
//   2. Init code that returns a single STOP byte deploys, and the runtime code
//      read back is 0x00.
//   3. SS58 to EVM. Ferdie pays 10 UNIT to Alith's fallback account, the H160
//      padded with 0xEE bytes, and Alith's EVM balance grows 10 UNIT.
//   4. EVM to SS58 by value. eth_estimateGas puts a plain transfer within the
//      21,000 gas wallets assume, and Alith pays 2 UNIT to Ferdie's H160 with
//      exactly that limit.
//   5. EVM to SS58 through 0x0802. Alith calls withdraw(dest, 5 UNIT) in an
//      EIP-1559 type-2 transaction to pay a fresh substrate account by its
//      AccountId32. The account also gets its existential deposit, so the
//      5 UNIT stay spendable in full.
// Every transfer reconciles exactly on both sides, with the fee from
// TransactionFeePaid on substrate and from the receipt on EVM.
//
// Returns 1 on success, 0 on failure (zombienet `js-script ... return is 1`).
//
// Usage: js-script ./evm-tooling.js

const { spawn } = require("child_process");
const { WebSocketProvider, Wallet, Contract, getAddress, keccak256 } = require("ethers");
const { hexToU8a, u8aConcat, u8aToHex } = require("@polkadot/util");
const { connect, disconnectAll, pair, sleep, submitExtrinsic } = require("../../js-scripts/lib");

// Built by scripts/build-node.sh. alice's --eth-rpc in zombienet.toml points
// at this port.
const ETH_RPC = "../../target/eth-rpc/bin/eth-rpc";
const ETH_RPC_PORT = 8545;

// Alith, the first dev key EVM tooling ships with, pre-funded by the
// integration genesis preset.
const ALITH_PK =
    "0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133";
const PRECOMPILE = "0x0000000000000000000000000000000000000802";
const ABI = ["function withdraw(bytes32,uint256) returns (bool)"];

const EXPECTED_CHAIN_ID = 320262n;
const GAS_SCALE = 360_000_000n; // GasScale in runtime/src/configs/evm.rs.
const TRANSFER_GAS = 21_000n; // What wallets assume a plain transfer costs.
const MULTIPLIER_ONE = 10n ** 18n; // FixedU128 accuracy.
const UNIT = 10n ** 18n; // runtime UNIT == 1e18, EVM uses 18 decimals.
const TOPUP = 10n * UNIT; // SS58 -> EVM amount.
const PAYBACK = 2n * UNIT; // EVM -> SS58 amount by value.
const WITHDRAW = 5n * UNIT; // EVM -> SS58 amount through 0x0802.

// Init bytecode that returns a 1-byte runtime: STOP (0x00).
const INIT_CODE = "0x6001600c60003960016000f300";
const EXPECTED_RUNTIME = "0x00";

// pallet-revive keeps an H160 with no substrate account of its own in the
// AccountId32 that pads it with 0xEE bytes.
function fallbackAccount(h160) {
    return u8aConcat(hexToU8a(h160), new Uint8Array(12).fill(0xee));
}

// A substrate account's H160 is the last 20 bytes of the keccak of its
// AccountId32.
function h160Of(accountId) {
    return getAddress(`0x${keccak256(accountId).slice(-40)}`);
}

// Zombienet nodes prune state, so eth-rpc keeps recent blocks in memory
// instead of syncing the chain from genesis.
function startEthRpc(nodeWsUri) {
    return spawn(ETH_RPC, [
        "--node-rpc-url", nodeWsUri,
        "--rpc-port", `${ETH_RPC_PORT}`,
        "--eth-pruning", "256",
        "--no-prometheus",
    ], { stdio: ["ignore", "inherit", "inherit"] });
}

async function waitForEthRpc(child) {
    let failure = null;
    child.once("error", (e) => { failure = e; });
    child.once("exit", (code, signal) => {
        failure ??= new Error(`eth-rpc exited with ${signal || code}`);
    });
    for (;;) {
        if (failure) throw failure;
        try {
            const res = await fetch(`http://127.0.0.1:${ETH_RPC_PORT}`, {
                method: "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_chainId", params: [] }),
            });
            if (res.ok) return;
        } catch (_) {}
        await sleep(1000);
    }
}

async function ethHead(provider) {
    return Number(await provider.send("eth_blockNumber", []));
}

async function waitForEthHead(provider, number) {
    while ((await ethHead(provider)) < number) await sleep(1000);
}

// The fee multiplier moves every block, so a price counts only when eth-rpc's
// head holds still around it.
async function gasPriceAtHead(provider) {
    for (;;) {
        const head = await ethHead(provider);
        const price = BigInt(await provider.send("eth_gasPrice", []));
        if ((await ethHead(provider)) === head) return { head, price };
    }
}

async function free(api, address) {
    return (await api.query.system.account(address)).data.free.toBigInt();
}

async function run(_zombie, networkInfo, _args) {
    let ethRpc;
    let provider;
    let api;
    try {
        const alice = networkInfo.nodesByName["alice"];
        ethRpc = startEthRpc(alice.wsUri);
        await waitForEthRpc(ethRpc);
        provider = new WebSocketProvider(alice.wsUri);
        api = await connect(networkInfo, "alice");

        // --- Step 1. Chain id and gas price. ---
        const chainId = (await provider.getNetwork()).chainId;
        if (chainId !== EXPECTED_CHAIN_ID) {
            console.error("📜", `  chainId ${chainId} != ${EXPECTED_CHAIN_ID}`);
            return 0;
        }
        const { head, price } = await gasPriceAtHead(provider);
        const headHash = await api.rpc.chain.getBlockHash(head);
        const multiplier =
            (await api.query.transactionPayment.nextFeeMultiplier.at(headHash)).toBigInt() / MULTIPLIER_ONE;
        if (price !== multiplier * GAS_SCALE) {
            console.error("📜", `  gasPrice ${price} at #${head} != multiplier ${multiplier} * ${GAS_SCALE}`);
            return 0;
        }
        console.log("📜", `  config ok: chainId=${chainId} gasPrice(#${head})=${price}`);

        const wallet = new Wallet(ALITH_PK, provider);

        // --- Step 2. Minimal deploy path. ---
        const deployTx = await wallet.sendTransaction({ data: INIT_CODE });
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

        // --- Step 3. SS58 -> EVM top-up. ---
        // Ferdie is pre-funded but neither validates nor mines, so its balance
        // moves only with the transfers here (alice/bob/charlie/dave all mine).
        const ferdie = await pair("//Ferdie");
        const sink = await pair("//EvmWithdrawSink");

        const alithEvmBefore = await provider.getBalance(wallet.address);
        const ferdieBefore = await free(api, ferdie.address);

        const { status, events } = await submitExtrinsic(
            api, ferdie,
            api.tx.balances.transferKeepAlive(fallbackAccount(wallet.address), TOPUP.toString()),
        );
        const topupBlock = status.isInBlock ? status.asInBlock : status.asFinalized;
        await waitForEthHead(provider, (await api.rpc.chain.getHeader(topupBlock)).number.toNumber());

        const alithEvmGain = (await provider.getBalance(wallet.address)) - alithEvmBefore;
        if (alithEvmGain !== TOPUP) {
            console.error("📜", `  Alith EVM gain ${alithEvmGain} != ${TOPUP}`);
            return 0;
        }
        const feePaid = events.find(({ event }) =>
            api.events.transactionPayment.TransactionFeePaid.is(event));
        if (!feePaid) {
            console.error("📜", "  transfer emitted no TransactionFeePaid");
            return 0;
        }
        const txFee = feePaid.event.data.actualFee.toBigInt();
        const ferdieSpent = ferdieBefore - (await free(api, ferdie.address));
        if (ferdieSpent !== TOPUP + txFee) {
            console.error("📜", `  Ferdie spent ${ferdieSpent} != ${TOPUP} + fee ${txFee}`);
            return 0;
        }
        console.log("📜", `  SS58->EVM ok: Alith +${alithEvmGain}, Ferdie -${ferdieSpent} (fee ${txFee})`);

        // --- Step 4. EVM -> SS58 by value, the way a wallet sends it. ---
        const payback = { to: h160Of(ferdie.publicKey), value: PAYBACK };
        const estimate = await provider.estimateGas({ ...payback, from: wallet.address });
        if (estimate > TRANSFER_GAS) {
            console.error("📜", `  a plain transfer estimates ${estimate} gas > ${TRANSFER_GAS}`);
            return 0;
        }
        const ferdieBeforePayback = await free(api, ferdie.address);
        const alithEvmBeforePayback = await provider.getBalance(wallet.address);

        const pTx = await wallet.sendTransaction({ ...payback, gasLimit: TRANSFER_GAS });
        const pReceipt = await pTx.wait();
        if (!pReceipt || pReceipt.status !== 1) {
            console.error("📜", `  payback failed: status=${pReceipt && pReceipt.status}`);
            return 0;
        }
        const ferdieGain = (await free(api, ferdie.address)) - ferdieBeforePayback;
        if (ferdieGain !== PAYBACK) {
            console.error("📜", `  Ferdie gain ${ferdieGain} != ${PAYBACK}`);
            return 0;
        }
        const alithPaid = alithEvmBeforePayback - (await provider.getBalance(wallet.address));
        if (alithPaid !== PAYBACK + pReceipt.fee) {
            console.error("📜", `  Alith paid ${alithPaid} != ${PAYBACK} + fee ${pReceipt.fee}`);
            return 0;
        }
        console.log("📜", `  EVM->SS58 ok by value: Ferdie +${ferdieGain}, Alith -${alithPaid} (estimate ${estimate}, used ${pReceipt.gasUsed} gas)`);

        // --- Step 5. EVM -> SS58 withdraw via 0x0802 (EIP-1559 type-2). ---
        const erc20 = new Contract(PRECOMPILE, ABI, wallet);
        const sinkBefore = await free(api, sink.address);
        const alithEvmBeforeWithdraw = await provider.getBalance(wallet.address);

        const wTx = await erc20.withdraw(u8aToHex(sink.publicKey), WITHDRAW, { type: 2 });
        const wReceipt = await wTx.wait();
        if (!wReceipt || wReceipt.status !== 1 || wReceipt.type !== 2) {
            console.error("📜", `  withdraw bad receipt: status=${wReceipt && wReceipt.status} type=${wReceipt && wReceipt.type}`);
            return 0;
        }
        const ed = api.consts.balances.existentialDeposit.toBigInt();
        const sinkGain = (await free(api, sink.address)) - sinkBefore;
        if (sinkGain !== WITHDRAW + ed) {
            console.error("📜", `  sink gain ${sinkGain} != ${WITHDRAW} + existential deposit ${ed}`);
            return 0;
        }
        const alithDrop = alithEvmBeforeWithdraw - (await provider.getBalance(wallet.address));
        if (alithDrop !== WITHDRAW + wReceipt.fee) {
            console.error("📜", `  Alith drop ${alithDrop} != ${WITHDRAW} + gasFee ${wReceipt.fee}`);
            return 0;
        }
        console.log("📜", `  EVM->SS58 ok (type-2): sink +${sinkGain}, Alith -${alithDrop} (gas ${wReceipt.fee})`);

        console.log("📜", "  all EVM tooling checks passed");
        return 1;
    } catch (e) {
        console.error("📜", `  ${e.stack || e.message}`);
        return 0;
    } finally {
        await disconnectAll(api ? [api] : []);
        if (provider) {
            try { await provider.destroy(); } catch (_) {}
        }
        if (ethRpc) ethRpc.kill("SIGKILL");
    }
}

module.exports = { run };
