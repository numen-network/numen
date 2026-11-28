# Build

This guide explains how to build and run the blockchain node using Rust.  
It covers the required dependencies, environment setup, and step-by-step build instructions.

## Quick Start (Ubuntu/Debian)

One block from a clean machine to a finished build. Enter your sudo password at the start, then walk away.

```bash
sudo apt update
sudo apt install -y git clang curl libssl-dev llvm libclang-dev libudev-dev make pkg-config protobuf-compiler

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
rustup default stable
rustup update
rustup target add wasm32v1-none
rustup component add rust-src

git clone https://github.com/numen-network/numen.git
cd numen
cargo build
```

The sections below cover the same steps in detail, plus other operating systems.

## Build Dependencies

### Ubuntu/Debian

Use a terminal shell to execute the following commands:

```bash
sudo apt update
sudo apt install -y git clang curl libssl-dev llvm libclang-dev libudev-dev make pkg-config protobuf-compiler
```

### Arch Linux

Run these commands from a terminal:

```bash
pacman -Syu --needed --noconfirm git curl clang llvm openssl pkgconf protobuf make
```

### Fedora

Run these commands from a terminal:

```bash
sudo dnf update
sudo dnf install git curl clang llvm-devel openssl-devel systemd-devel make pkgconf-pkg-config protobuf-compiler
```

### OpenSUSE

Run these commands from a terminal:

```bash
sudo zypper install git curl clang llvm-devel openssl-devel libudev-devel make pkg-config protobuf
```

### macOS

> **Apple M1/M2 ARM** If you have an Apple M1/M2 ARM system on a chip, make sure that you have Apple Rosetta 2 installed
> through `softwareupdate --install-rosetta`. This is only needed to run the `protoc` tool during the build. The build
> itself and the target binaries would remain native.

Open the Terminal application and execute the following commands:

```bash
# Install Homebrew if necessary https://brew.sh/
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/master/install.sh)"

# Make sure Homebrew is up-to-date, install dependencies
brew update
brew install openssl pkg-config protobuf
```

### Windows

Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the C++ desktop workload. Install Git, LLVM, Protoc and Strawberry Perl too. Turn off Git CRLF conversion and NTFS path checks before cloning.

```powershell
choco install llvm protoc strawberryperl -y
git config --global core.autocrlf false
git config --global core.protectNTFS false
```

Set these environment variables in PowerShell before running Cargo.

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
$env:OPENSSL_SRC_PERL = "C:\Strawberry\perl\bin\perl.exe"
$env:CL = "/utf-8"
$env:CARGO_NET_GIT_FETCH_WITH_CLI = "true"
```

If Cargo rejects the `aux` path in `polkadot-sdk`, run the checkout step from [`.github/actions/windows-dependencies/action.yml`](../.github/actions/windows-dependencies/action.yml) in Git Bash. WSL2 with the [Ubuntu/Debian](#ubuntudebian) steps also works.

---

## Rust Developer Environment

### 1. Install rustup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
```

### 2. Install the stable toolchain and add the WASM target

```bash
rustup default stable
rustup update
rustup target add wasm32v1-none
rustup component add rust-src
```

> **Note**: Since Rust 1.84, `wasm32v1-none` is the recommended target over the legacy `wasm32-unknown-unknown`.
> `wasm32v1-none` is designed for bare-metal WASM environments without OS assumptions, making it a better fit for blockchain runtimes.
> **A nightly toolchain is no longer required.**

### 3. Verify your setup

```bash
rustup show
```

Expected output:

```text
Default host: x86_64-unknown-linux-gnu

installed targets for active toolchain
--------------------------------------
wasm32v1-none
x86_64-unknown-linux-gnu

active toolchain
----------------
stable-x86_64-unknown-linux-gnu (default)
rustc 1.84.0 (...)
```

---

## Building the Project

```bash
git clone https://github.com/numen-network/numen.git
cd numen
cargo build
```

The first build compiles all dependencies and may take 20–60 minutes.

### Minimum Hardware Requirements

| Component | Minimum |
|-----------|---------|
| RAM       |    6 GB |
| Disk      |   12 GB |

### Release Build

The published binaries are built differently. Use this if you want to reproduce one.

```bash
cargo build --profile production --locked -p numen --features metadata-hash
```

The binary lands in `target/production/numen`.

Two things set it apart from a plain `cargo build`.

The `production` profile turns on fat LTO and a single codegen unit. Faster node, slower build, more memory.

The `metadata-hash` feature bakes the runtime metadata hash into the wasm. Hardware wallets like Ledger and Polkadot Vault need it to verify what they sign. Costs a second wasm build.

Expect well over an hour on a cold cache.

### eth-rpc

The release archive also carries eth-rpc. It's built from the polkadot-sdk commit `Cargo.lock` pins, the same one the runtime uses.

```bash
.maintain/build-eth-rpc.sh target/eth-rpc --profile production
```

The binary lands in `target/eth-rpc/bin/eth-rpc`.
