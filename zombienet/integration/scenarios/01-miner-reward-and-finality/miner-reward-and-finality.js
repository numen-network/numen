// "<m1,m2,...>,<h>"

const { connect, POW_ENGINE_ID, getUri, waitBlockAt, waitFinalizedAt } = require("../../js-scripts/lib");
const { Keyring } = require("@polkadot/keyring");
const { decodeAddress, cryptoWaitReady } = require("@polkadot/util-crypto");
const { u8aToHex } = require("@polkadot/util");

async function connectAll(networkInfo, nodeNames) {
    const apis = {};
    for (const name of nodeNames) apis[name] = await connect(networkInfo, name);
    return apis;
}

async function waitAllFinalized(apis, height) {
    for (const [name, api] of Object.entries(apis)) {
        await waitFinalizedAt(api, height);
        console.log("📜", `  ${name.padEnd(8)} finalized >= #${height}`);
    }
}

async function assertFinalizedConsistency(apis, height) {
    const nodeNames = Object.keys(apis);
    for (let h = 1; h <= height; h += 1) {
        let reference = null;
        let referenceFrom = null;
        for (const name of nodeNames) {
            const hash = (await apis[name].rpc.chain.getBlockHash(h)).toHex();
            if (reference === null) {
                reference = hash;
                referenceFrom = name;
            } else if (hash !== reference) {
                throw new Error(`height #${h}: ${name}=${hash} != ${referenceFrom}=${reference}`);
            }
        }
        console.log("📜", `  #${h} hash = ${reference}`);
    }
}

function buildWatch(names, keyring) {
    const watch = {};
    for (const name of names) {
        const kp = keyring.addFromUri(getUri(name));
        watch[name] = {
            address: kp.address,
            addressHex: u8aToHex(decodeAddress(kp.address)).toLowerCase(),
            blocks: 0,
        };
    }
    const byHex = Object.fromEntries(
        Object.entries(watch).map(([n, v]) => [v.addressHex, n])
    );
    return { watch, byHex };
}

async function snapshotBalances(alice, watch, height) {
    const hash0 = await alice.rpc.chain.getBlockHash(0);
    const hashN = await alice.rpc.chain.getBlockHash(height);
    const apiAt0 = await alice.at(hash0);
    const apiAtN = await alice.at(hashN);
    for (const [name, v] of Object.entries(watch)) {
        v.init = (await apiAt0.query.system.account(v.address)).data.free.toBigInt();
        v.final = (await apiAtN.query.system.account(v.address)).data.free.toBigInt();
        console.log("📜", `  ${name.padEnd(8)} init@#0=${v.init} final@#${height}=${v.final}`);
    }
}

async function countAuthoredBlocks(alice, watch, byHex, height) {
    let watched = 0;
    let unwatched = 0;
    for (let h = 1; h <= height; h += 1) {
        const hash = await alice.rpc.chain.getBlockHash(h);
        const header = await alice.rpc.chain.getHeader(hash);
        let authorHex = null;
        for (const log of header.digest.logs) {
            if (log.isPreRuntime) {
                const [engine, data] = log.asPreRuntime;
                if (engine.toHex() === POW_ENGINE_ID) {
                    authorHex = u8aToHex(data.slice(0, 32)).toLowerCase();
                    break;
                }
            }
        }
        const who = authorHex ? byHex[authorHex] : null;
        if (who) {
            watch[who].blocks += 1;
            watched += 1;
            console.log("📜", `  #${h} -> ${who}`);
        } else {
            unwatched += 1;
            console.log("📜", `  #${h} -> (unwatched ${authorHex || "no-digest"})`);
        }
    }

    console.log("📜", `  authored #1..#${height}: watched=${watched}, unwatched=${unwatched}`);
}

function assertRewards(watch, reward) {
    for (const [name, v] of Object.entries(watch)) {
        const delta = v.final - v.init;
        const expected = BigInt(v.blocks) * reward;
        const tag = delta === expected ? "ok" : "MISMATCH";
        console.log("📜", `  ${name.padEnd(8)} blocks=${v.blocks} delta=${delta} expected=${expected}  [${tag}]`);
        if (delta !== expected) throw new Error(`miner reward reconciliation failed`);
    }
}

async function run(_zombie, networkInfo, args) {
    if (!args || args.length < 2) {
        console.error("📜", `  usage: with "<m1>,<m2>,...,<h>"; got args=${JSON.stringify(args)}`);
        return 0;
    }
    const height = Number(args[args.length - 1]);
    const names = args.slice(0, args.length - 1).map((s) => String(s).trim()).filter(Boolean);
    if (!Number.isFinite(height) || height < 1 || names.length === 0) {
        console.error("📜", `  bad args: miners=${JSON.stringify(names)} h=${height}`);
        return 0;
    }

    await cryptoWaitReady();
    const keyring = new Keyring({ type: "sr25519" });

    const nodeNames = Object.keys(networkInfo.nodesByName);
    console.log("📜", `  nodes = ${nodeNames.join(",")}`);
    console.log("📜", `  watched miners = ${names.join(",")}, target height = #${height}`);

    const apis = await connectAll(networkInfo, nodeNames);
    try {
        const alice = apis.alice || apis[nodeNames[0]];

        await waitBlockAt(alice, height);
        await waitAllFinalized(apis, height);
        await assertFinalizedConsistency(apis, height);

        // Reward halves every HalvingInterval blocks; integration runs stay far
        // below the first boundary, so every block pays the initial reward.
        const reward = alice.consts.blockReward.initialReward.toBigInt();
        console.log("📜", `  reward/block = ${reward}`);

        const { watch, byHex } = buildWatch(names, keyring);
        await snapshotBalances(alice, watch, height);
        await countAuthoredBlocks(alice, watch, byHex, height);
        assertRewards(watch, reward);

        return 1;
    } catch (e) {
        console.error("📜", `  ${e.message}`);
        return 0;
    } finally {
        for (const a of Object.values(apis)) await a.disconnect();
    }
}

module.exports = { run };
