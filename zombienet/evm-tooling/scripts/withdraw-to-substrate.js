// EVM -> Substrate withdrawal through the ERC20 token at 0x0802.
//
//   node withdraw-to-substrate.js
//
// Sends `amount` NUMN from Alith to a target substrate AccountId32 by
// invoking `withdraw(bytes32 dest, uint256 amount)` on the token.
//
// Verification is EVM-side only (caller balance decreased and the
// `Withdrawal(address,bytes32,uint256)` event matches). Substrate-side
// verification through `@polkadot/api` lives in
// `zombienet/integration/scenarios/02-evm-tooling/evm-tooling.js`.

const {
  JsonRpcProvider,
  Wallet,
  Contract,
  formatEther,
  parseEther,
  keccak256,
  toUtf8Bytes,
  zeroPadValue,
  hexlify,
  getAddress,
} = require("ethers");

const RPC_URL = process.env.NUMEN_RPC || "http://127.0.0.1:9944";
const ALITH_PK =
  process.env.ALITH_PK ||
  "0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133";
const PRECOMPILE = "0x0000000000000000000000000000000000000802";

// Alice's well-known sr25519 dev account (`//Alice`) as a 32-byte public key.
const ALICE_PUBKEY =
  process.env.SUBSTRATE_DEST ||
  "0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d";
const AMOUNT = parseEther(process.env.AMOUNT || "2.5");
const EXPECTED_CHAIN_ID = 320262n;

const ABI = [
  "function balanceOf(address) view returns (uint256)",
  "function withdraw(bytes32,uint256) returns (bool)",
  "event Withdrawal(address indexed src, bytes32 indexed dest, uint256 wad)",
];

const WITHDRAWAL_TOPIC = keccak256(toUtf8Bytes("Withdrawal(address,bytes32,uint256)"));

async function main() {
  const provider = new JsonRpcProvider(RPC_URL);
  const network = await provider.getNetwork();
  console.log(`chainId:         ${network.chainId}`);
  if (network.chainId !== EXPECTED_CHAIN_ID) {
      console.error(`unexpected chainId ${network.chainId}, want ${EXPECTED_CHAIN_ID}`);
      process.exit(1);
  }

  const wallet = new Wallet(ALITH_PK, provider);
  const erc20 = new Contract(PRECOMPILE, ABI, wallet);

  console.log(`from(EVM):       ${wallet.address}`);
  console.log(`to(substrate):   ${ALICE_PUBKEY}`);
  console.log(`amount:          ${formatEther(AMOUNT)} NUMN`);

  const before = await erc20.balanceOf(wallet.address);
  console.log(`before alith:    ${formatEther(before)} NUMN`);

  const tx = await erc20.withdraw(ALICE_PUBKEY, AMOUNT);
  console.log(`tx hash:         ${tx.hash}`);
  const receipt = await tx.wait();
  console.log(`status:          ${receipt.status === 1 ? "ok" : "failed"} (block ${receipt.blockNumber})`);
  if (receipt.status !== 1) {
    process.exit(1);
  }

  // Locate and validate the Withdrawal log emitted by the precompile.
  const log = receipt.logs.find(
    (l) => l.address.toLowerCase() === PRECOMPILE.toLowerCase() && l.topics[0] === WITHDRAWAL_TOPIC
  );
  if (!log) {
    console.error("Withdrawal log not found in receipt");
    process.exit(1);
  }
  const srcTopic = getAddress("0x" + log.topics[1].slice(26));
  const destTopic = log.topics[2];
  const wad = BigInt(log.data);
  console.log(`event Withdrawal src=${srcTopic} dest=${destTopic} wad=${formatEther(wad)} NUMN`);

  if (srcTopic !== getAddress(wallet.address)) {
    console.error(`event src mismatch: got ${srcTopic}, want ${wallet.address}`);
    process.exit(1);
  }
  if (destTopic.toLowerCase() !== zeroPadValue(hexlify(ALICE_PUBKEY), 32).toLowerCase()) {
    console.error(`event dest mismatch: got ${destTopic}, want ${ALICE_PUBKEY}`);
    process.exit(1);
  }
  if (wad !== AMOUNT) {
    console.error(`event wad mismatch: got ${wad}, want ${AMOUNT}`);
    process.exit(1);
  }

  const after = await erc20.balanceOf(wallet.address);
  console.log(`after alith:     ${formatEther(after)} NUMN`);
  // After the call: balance must drop by at least `AMOUNT` (the rest is gas).
  if (before - after < AMOUNT) {
    console.error(`alith delta ${formatEther(before - after)} NUMN < requested ${formatEther(AMOUNT)} NUMN`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
