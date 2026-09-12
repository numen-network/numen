<div align="center">

# Numen

A Layer 1 blockchain powered by Proof of Scan, built with the Polkadot SDK.

![GitHub stars](https://img.shields.io/github/stars/numen-network/numen)&nbsp;&nbsp;![GitHub forks](https://img.shields.io/github/forks/numen-network/numen)

[![CI](https://img.shields.io/github/actions/workflow/status/numen-network/numen/ci.yml?label=CI)](https://github.com/numen-network/numen/actions/workflows/ci.yml)&nbsp;&nbsp;[![License](https://img.shields.io/github/license/numen-network/numen)](LICENSE)&nbsp;&nbsp;[![Last commit](https://img.shields.io/github/last-commit/numen-network/numen)](https://github.com/numen-network/numen/commits/master)&nbsp;&nbsp;[![Discord](https://img.shields.io/discord/1528532360113684590?logo=discord&label=Discord)](https://discord.gg/WKmyTfmaa)&nbsp;&nbsp;[![Website](https://img.shields.io/badge/Website-numen--network.org-blue)](https://www.numen-network.org)

</div>

---

## Requirements

|             | CPU     | RAM  | Disk  |
| ----------- | ------- | ---- | ----- |
| Minimum     | 1 core  | 1 GB | 5 GB  |
| Recommended | 4 cores | 4 GB | 20 GB |

Disk usage grows as the chain does.

## Getting Started

Grab a prebuilt binary from the [releases page](https://github.com/numen-network/numen/releases). Each archive holds the `numen` binary and `mainnet-raw.json`. Nothing else to download.

```bash
tar -xzf numen-linux-x86_64.tar.gz
cd numen-linux-x86_64
```

Builds ship for Linux x86_64 and macOS arm64. Build from source for anything else, or to track master. See [docs/how-to-build.md](docs/how-to-build.md).

For the testnet, download [testnet-raw.json](https://github.com/numen-network/numen/raw/refs/heads/master/testnet-raw.json) and pass it to `--chain` instead of `mainnet`.

## Run a node

> Recommend opening port 30333 on your firewall to help strengthen the network.
> Open UDP 123 both ways as well, or NTP never gets through. Drift past 2 seconds and the node rejects new blocks, stuck on its own fork.

Run it the simplest way

```bash
./numen --chain mainnet-raw.json
```

Run an archive node to keep every historical state

```bash
./numen --chain mainnet-raw.json --state-pruning archive
```

Run it as a service

```ini
[Unit]
Description=Numen node
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
ExecStart=/opt/numen/numen \
    --chain /opt/numen/mainnet-raw.json \
    --in-peers 100 \
    --out-peers 25 \
    --in-peers-light 50
Restart=always
RestartSec=5s
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload && systemctl enable --now numen
```

Logs land in the journal.

```bash
journalctl -u numen -f
```

## Mining

Mine locally and credit rewards to the given account.

```bash
./numen --chain mainnet-raw.json --miner <YOUR_ADDRESS> --node-miner <THREADS>
```

`--miner` sets the reward address and exposes the mining RPC so external miners can scan off the node and submit seals.
Pull the current task with `mining_getTask` or subscribe to `mining_subscribeTask` for a fresh task pushed every second, then return a found seal with `mining_submitSeal`. 
Add `--node-miner` to also run the in-process scan loop across every core, or `--node-miner <THREADS>` to cap the scan threads.
Drop it to leave block authoring entirely to external miners.

Mining never needs a private key. The node only puts the payout `AccountId` into the block header, 
so generate a keypair offline (e.g. with `subkey generate`) and pass only the SS58 address to the mining node. 
Keep the private key on a separate, offline machine.

If the address is invalid SS58 the node refuses to start.