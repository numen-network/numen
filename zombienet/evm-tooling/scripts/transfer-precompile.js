// Native NUMN transfer through the ERC20 token at 0x0802.
//
//   node transfer-precompile.js
//
// Defaults: Alith -> Baltathar, 1.5 NUMN, RPC at 127.0.0.1:9944.
//
// Unlike `transfer-ethers.js` (which submits a vanilla EVM value tx), this
// script invokes `transfer(address,uint256)` on the token. The native
// balance moves the same way, but the call path proves that wallets and
// contracts treating NUMN as ERC20 work against the chain.

const { JsonRpcProvider, Wallet, Contract, formatEther, parseEther } = require("ethers");

const RPC_URL = process.env.NUMEN_RPC || "http://127.0.0.1:9944";
const ALITH_PK =
  process.env.ALITH_PK ||
  "0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133";
const BALTATHAR = "0x3Cd0A705a2DC65e5b1E1205896BaA2be8A07c6e0";
const PRECOMPILE = "0x0000000000000000000000000000000000000802";
const AMOUNT = parseEther(process.env.AMOUNT || "1.5");
const EXPECTED_CHAIN_ID = 320262n;

const ERC20_ABI = [
  "function name() view returns (string)",
  "function symbol() view returns (string)",
  "function decimals() view returns (uint8)",
  "function totalSupply() view returns (uint256)",
  "function balanceOf(address) view returns (uint256)",
  "function transfer(address,uint256) returns (bool)",
];

async function main() {
  const provider = new JsonRpcProvider(RPC_URL);
  const network = await provider.getNetwork();
  console.log(`chainId: ${network.chainId}`);
  if (network.chainId !== EXPECTED_CHAIN_ID) {
      console.error(`unexpected chainId ${network.chainId}, want ${EXPECTED_CHAIN_ID}`);
      process.exit(1);
  }

  const wallet = new Wallet(ALITH_PK, provider);
  const erc20 = new Contract(PRECOMPILE, ERC20_ABI, wallet);

  const [name, symbol, decimals] = await Promise.all([
    erc20.name(),
    erc20.symbol(),
    erc20.decimals(),
  ]);
  console.log(`token:   ${name} (${symbol}, ${decimals} decimals) @ ${PRECOMPILE}`);
  if (symbol !== "NUMN" || Number(decimals) !== 18) {
    console.error(`unexpected metadata: symbol=${symbol} decimals=${decimals}`);
    process.exit(1);
  }

  const beforeFromErc = await erc20.balanceOf(wallet.address);
  const beforeFromNat = await provider.getBalance(wallet.address);
  const beforeTo = await erc20.balanceOf(BALTATHAR);
  console.log(
    `before:  alith=${formatEther(beforeFromErc)} (eth_getBalance=${formatEther(beforeFromNat)}) baltathar=${formatEther(beforeTo)}`
  );
  if (beforeFromErc !== beforeFromNat) {
    console.error("balanceOf and eth_getBalance disagree before transfer");
    process.exit(1);
  }

  const tx = await erc20.transfer(BALTATHAR, AMOUNT);
  console.log(`tx hash: ${tx.hash}`);
  const receipt = await tx.wait();
  console.log(`status:  ${receipt.status === 1 ? "ok" : "failed"} (block ${receipt.blockNumber})`);
  if (receipt.status !== 1) {
    process.exit(1);
  }

  const afterTo = await erc20.balanceOf(BALTATHAR);
  console.log(`after:   baltathar=${formatEther(afterTo)}`);
  if (afterTo - beforeTo !== AMOUNT) {
    console.error("recipient delta does not match transfer amount");
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
