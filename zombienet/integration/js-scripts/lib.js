// Shared CommonJS helpers for zombienet js-script blocks.
//
// Every js-script in this directory receives `(zombie, networkInfo, args)`
// from zombienet, where `networkInfo.nodesByName[<name>].wsUri` is the
// dynamically allocated WebSocket endpoint. These helpers wrap that pattern
// and expose a small set of common operations against any node.

const { ApiPromise, WsProvider } = require("@polkadot/api");
const { Keyring } = require("@polkadot/keyring");
const { cryptoWaitReady, decodeAddress } = require("@polkadot/util-crypto");
const { u8aToHex } = require("@polkadot/util");

async function connect(networkInfo, nodeName) {
    const info = networkInfo.nodesByName[nodeName];
    if (!info) throw new Error(`unknown node: ${nodeName}`);
    const provider = new WsProvider(info.wsUri);
    const api = await ApiPromise.create({
        provider,
        throwOnConnect: true,
        noInitWarn: true,
        types: {
            GeneratedSessionKeys: {
                keys: "Bytes",
                proof: "Bytes",
            },
        },
        rpc: {
            author: {
                rotateKeysWithOwner: {
                    description: "Generate new session keys and a matching ownership proof",
                    params: [{ name: "owner", type: "Bytes" }],
                    type: "GeneratedSessionKeys",
                },
            },
        },
    });
    return api;
}

async function connectAll(networkInfo, names) {
    return Promise.all(names.map((n) => connect(networkInfo, n)));
}

async function disconnectAll(apis) {
    for (const api of apis) {
        try { await api.disconnect(); } catch (_) {}
    }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function getUri(name) {
    return `//${name[0].toUpperCase()}${name.slice(1).toLowerCase()}`;
}

async function finalizedNumber(api) {
    const hash = await api.rpc.chain.getFinalizedHead();
    const header = await api.rpc.chain.getHeader(hash);
    return header.number.toNumber();
}

function watchStream(subscribe, onHead, timeoutBlocks) {
    return new Promise((resolve, reject) => {
        let unsub = null;
        let firstNum = null;
        const stop = (fn, v) => { if (unsub) { unsub(); unsub = null; } fn(v); };
        subscribe(async (header) => {
            try {
                const num = header.number.toNumber();
                if (firstNum === null) firstNum = num;
                const v = await onHead(header);
                if (v !== undefined) return stop(resolve, v);
                if (timeoutBlocks !== undefined && num - firstNum >= timeoutBlocks) {
                    return stop(reject, new Error(`watchStream: ${timeoutBlocks} blocks elapsed (#${firstNum}..#${num}) without result`));
                }
            } catch (e) {
                stop(reject, e);
            }
        }).then((u) => { unsub = u; }).catch(reject);
    });
}

async function watchHeads(api, onHead, timeoutBlocks) {
    return watchStream((cb) => api.rpc.chain.subscribeNewHeads(cb), onHead, timeoutBlocks);
}

async function watchFinalizedHeads(api, onHead, timeoutBlocks) {
    return watchStream((cb) => api.rpc.chain.subscribeFinalizedHeads(cb), onHead, timeoutBlocks);
}

async function waitBlock(api, num) {
    let seen = 0;
    return watchHeads(api, () => {
        seen += 1;
        if (seen > num) return true;
    });
}

async function waitBlockAt(api, height) {
    if ((await api.rpc.chain.getHeader()).number.toNumber() >= height) return;
    return watchHeads(api, (header) => {
        if (header.number.toNumber() >= height) return true;
    });
}

async function waitFinalizedAt(api, height) {
    if (await finalizedNumber(api) >= height) return;
    return watchFinalizedHeads(api, (header) => {
        if (header.number.toNumber() >= height) return true;
    });
}

function submitExtrinsic(api, signer, extrinsic) {
    return new Promise((resolve, reject) => {
        extrinsic
            .signAndSend(signer, ({ status, dispatchError, events }) => {
                if (dispatchError) {
                    let msg = dispatchError.toString();
                    if (dispatchError.isModule) {
                        const decoded = api.registry.findMetaError(dispatchError.asModule);
                        msg = `${decoded.section}.${decoded.name}: ${decoded.docs.join(" ")}`;
                    }
                    return reject(new Error(`extrinsic failed: ${msg}`));
                }
                if (status.isInBlock || status.isFinalized) resolve({ status, events });
            })
            .catch(reject);
    });
}

// ------------ Account ------------

let _keyring;
async function keyring() {
    if (_keyring) return _keyring;
    await cryptoWaitReady();
    _keyring = new Keyring({ type: "sr25519", ss58Format: 42 });
    return _keyring;
}

async function pair(uri) {
    const k = await keyring();
    return k.addFromUri(uri);
}

function addressHex(ss58OrBytes) {
    return u8aToHex(decodeAddress(ss58OrBytes)).toLowerCase();
}

// ------------ PoW ------------

const POW_ENGINE_ID = "0x706f775f"; // b"pow_"

function powAuthorHexFromHeader(header) {
    for (const log of header.digest.logs) {
        if (log.isPreRuntime) {
            const [engine, data] = log.asPreRuntime;
            if (engine.toHex() === POW_ENGINE_ID) {
                return u8aToHex(data.slice(0, 32)).toLowerCase();
            }
        }
    }
    return null;
}

// ------------ Session ------------

async function sessionContainsValidator(api, address) {
    const set = await api.query.session.validators();
    return set.some((id) => id.toString() === address);
}

async function waitForSessionRotations(api, n) {
    const start = (await api.query.session.currentIndex()).toNumber();
    const target = start + n;
    return watchHeads(api, async () => {
        const cur = (await api.query.session.currentIndex()).toNumber();
        if (cur >= target) return cur;
    });
}

// ------------------------

module.exports = {
    connect, connectAll, disconnectAll,
    sleep, getUri,
    finalizedNumber, sessionContainsValidator,
    waitForSessionRotations, watchHeads, watchFinalizedHeads,
    waitBlock, waitBlockAt, waitFinalizedAt,
    keyring, pair, submitExtrinsic,
    POW_ENGINE_ID, addressHex, powAuthorHexFromHeader
};
