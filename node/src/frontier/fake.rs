//! A node made of a few headers and the runtime answers handed to it.

use std::{
	cell::RefCell,
	collections::{BTreeMap, HashMap},
	sync::{
		atomic::{AtomicU32, AtomicUsize, Ordering},
		Mutex,
	},
};

use codec::Encode;
use ethereum::{
	eip2930::TransactionSignature, BlockV3, EIP1559Transaction, EIP658ReceiptData, Log, PartialHeader, ReceiptV4,
	TransactionAction, TransactionV3,
};
use ethereum_types::Bloom;
use serde_json::Value;
use sp_api::ApiError;
use sp_core::{H160, H256, U256};
use sp_runtime::{
	generic::{Digest, DigestItem},
	traits::Header as _,
};

use numen_runtime::opaque::Header;

use super::{
	index::FRONTIER_ENGINE_ID,
	node::{Changes, Node},
	RUNTIME_API_VERSION,
};

pub const CURRENT_ALL: &str = "EthereumRuntimeRPCApi_current_all";
pub const CURRENT_BLOCK: &str = "EthereumRuntimeRPCApi_current_block";
pub const GAS_PRICE: &str = "EthereumRuntimeRPCApi_gas_price";
pub const ACCOUNT_BASIC: &str = "EthereumRuntimeRPCApi_account_basic";
pub const ACCOUNT_CODE_AT: &str = "EthereumRuntimeRPCApi_account_code_at";
pub const MULTIPLIER_SUPPORT: &str = "EthereumRuntimeRPCApi_gas_limit_multiplier_support";
pub const CALL: &str = "EthereumRuntimeRPCApi_call";
pub const CREATE: &str = "EthereumRuntimeRPCApi_create";

/// The gas limit of the Ethereum blocks the fake chain holds.
pub const BLOCK_GAS_LIMIT: u64 = 75_000_000;

/// Who sends the transactions the tests make up.
pub const SENDER: H160 = H160::repeat_byte(0xaa);

/// A runtime call the node got, with the storage changes it left behind.
pub struct Called {
	pub function: &'static str,
	pub arguments: Vec<u8>,
	pub changes: Changes,
	pub metered: bool,
}

/// Answers a runtime call from its arguments.
type Responder = Box<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>;

pub struct FakeNode {
	pub best: u32,
	finalized: AtomicU32,
	pub api_version: Option<u32>,
	headers: BTreeMap<u32, Header>,
	numbers: HashMap<H256, u32>,
	answers: HashMap<(H256, String, Vec<u8>), Vec<u8>>,
	responders: HashMap<(H256, &'static str), Responder>,
	/// What a runtime function writes to storage when called.
	writes: HashMap<&'static str, (Vec<u8>, Vec<u8>)>,
	storage: HashMap<(H256, Vec<u8>), Vec<u8>>,
	aux: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
	pub aux_writes: AtomicUsize,
	pub called: Mutex<Vec<Called>>,
}

impl FakeNode {
	/// A chain of just the genesis block, which holds an empty Ethereum block.
	pub fn new() -> Self {
		let mut node = Self {
			best: 0,
			finalized: AtomicU32::new(0),
			api_version: Some(RUNTIME_API_VERSION),
			headers: BTreeMap::new(),
			numbers: HashMap::new(),
			answers: HashMap::new(),
			responders: HashMap::new(),
			writes: HashMap::new(),
			storage: HashMap::new(),
			aux: Mutex::new(BTreeMap::new()),
			aux_writes: AtomicUsize::new(0),
			called: Mutex::new(Vec::new()),
		};
		node.insert(0, Digest::default());
		node.hold(0, U256::one(), Vec::new());
		node
	}

	/// A chain recorded off mainnet, the numbers of its blocks and the answers
	/// the Frontier node gave about them.
	pub fn mainnet() -> (Self, Vec<u32>, Vec<Value>) {
		let fixture: Value = serde_json::from_str(include_str!("fixtures/mainnet.json")).unwrap();
		let bytes = |value: &Value| sp_core::bytes::from_hex(value.as_str().unwrap()).unwrap();
		let mut node = Self::new();
		let mut numbers = Vec::new();
		for block in fixture["blocks"].as_array().unwrap() {
			let number = block["number"].as_u64().unwrap() as u32;
			numbers.push(number);
			let transactions = block["transactions"].as_array().unwrap();
			let hashes = transactions.iter().map(|hash| H256::from_slice(&bytes(hash))).collect();
			node.frontier_block(number, H256::from_slice(&bytes(&block["ethereum_hash"])), hashes);
			node.answer(number, CURRENT_ALL, Vec::new(), bytes(&block["current_all"]));
			node.answer(number, GAS_PRICE, Vec::new(), bytes(&block["gas_price"]));
		}
		for call in fixture["calls"].as_array().unwrap() {
			let number = call["at"].as_u64().unwrap() as u32;
			node.answer(number, call["function"].as_str().unwrap(), bytes(&call["arguments"]), bytes(&call["result"]));
		}
		(node, numbers, fixture["answers"].as_array().unwrap().clone())
	}

	fn insert(&mut self, number: u32, digest: Digest) -> H256 {
		let parent = self.headers.get(&number.wrapping_sub(1)).map(Header::hash).unwrap_or_default();
		let header = Header::new(number, H256::zero(), H256::zero(), parent, digest);
		let hash = header.hash();
		self.headers.insert(number, header);
		self.numbers.insert(hash, number);
		self.best = self.best.max(number);
		hash
	}

	/// Adds a block that carries these Ethereum hashes in its header.
	pub fn frontier_block(&mut self, number: u32, block_hash: H256, transaction_hashes: Vec<H256>) -> H256 {
		let hashes = (1u8, block_hash, transaction_hashes).encode();
		self.insert(number, Digest { logs: vec![DigestItem::Consensus(FRONTIER_ENGINE_ID, hashes)] })
	}

	/// Adds a block Frontier did not build.
	pub fn revive_block(&mut self, number: u32) -> H256 {
		self.insert(number, Digest::default())
	}

	/// Makes the runtime of block `number` answer a call.
	pub fn answer(&mut self, number: u32, function: &str, arguments: Vec<u8>, result: Vec<u8>) {
		let at = self.headers[&number].hash();
		self.answers.insert((at, function.to_owned(), arguments), result);
	}

	/// Makes the runtime of block `number` work out the answer to a call from
	/// its arguments.
	pub fn answer_with(
		&mut self,
		number: u32,
		function: &'static str,
		responder: impl Fn(&[u8]) -> Vec<u8> + Send + Sync + 'static,
	) {
		self.responders.insert((self.headers[&number].hash(), function), Box::new(responder));
	}

	pub fn finalize(&self, number: u32) {
		self.finalized.store(number, Ordering::Relaxed);
	}

	/// Makes a runtime function write to storage whenever it is called.
	pub fn write_on(&mut self, function: &'static str, key: &[u8], value: &[u8]) {
		self.writes.insert(function, (key.to_vec(), value.to_vec()));
	}

	pub fn set_storage(&mut self, number: u32, key: Vec<u8>, value: Vec<u8>) {
		self.storage.insert((self.headers[&number].hash(), key), value);
	}

	/// Makes block `number` hold an Ethereum block of these transactions, each
	/// with what its receipt records. A block not on the chain yet is added as
	/// a Frontier block.
	pub fn hold(&mut self, number: u32, base_fee: U256, transactions: Vec<(TransactionV3, EIP658ReceiptData)>) -> BlockV3 {
		let header = PartialHeader {
			parent_hash: H256::repeat_byte(number as u8),
			beneficiary: H160::zero(),
			state_root: H256::zero(),
			receipts_root: H256::zero(),
			logs_bloom: Bloom::zero(),
			difficulty: U256::zero(),
			number: number.into(),
			gas_limit: BLOCK_GAS_LIMIT.into(),
			gas_used: U256::zero(),
			timestamp: 1_000 * u64::from(number),
			extra_data: Vec::new(),
			mix_hash: H256::zero(),
			nonce: Default::default(),
		};
		let (signed, outcomes): (Vec<_>, Vec<_>) = transactions.into_iter().unzip();
		let block = BlockV3::new(header, signed, Vec::new());
		if !self.headers.contains_key(&number) {
			self.frontier_block(number, block.header.hash(), block.transactions.iter().map(TransactionV3::hash).collect());
		}
		let statuses: Vec<_> = block
			.transactions
			.iter()
			.zip(&outcomes)
			.enumerate()
			.map(|(index, (signed, outcome))| {
				(signed.hash(), index as u32, SENDER, None::<H160>, None::<H160>, outcome.logs.clone(), Bloom::zero())
			})
			.collect();
		let receipts: Vec<_> = outcomes.into_iter().map(ReceiptV4::EIP1559).collect();
		self.answer(number, CURRENT_ALL, Vec::new(), (Some(&block), Some(receipts), Some(statuses)).encode());
		self.answer(number, CURRENT_BLOCK, Vec::new(), Some(&block).encode());
		self.answer(number, GAS_PRICE, Vec::new(), base_fee.encode());
		block
	}

	pub fn aux_entries(&self) -> usize {
		self.aux.lock().unwrap().len()
	}
}

/// A dynamic fee transaction of the given tip and fee cap.
pub fn transaction(nonce: u64, tip: u64, cap: u64) -> TransactionV3 {
	TransactionV3::EIP1559(EIP1559Transaction {
		chain_id: 32026,
		nonce: nonce.into(),
		max_priority_fee_per_gas: tip.into(),
		max_fee_per_gas: cap.into(),
		gas_limit: U256::from(100_000),
		action: TransactionAction::Call(H160::repeat_byte(0xbb)),
		value: U256::zero(),
		input: Vec::new(),
		access_list: Vec::new(),
		signature: TransactionSignature::new(false, H256::from_low_u64_be(1), H256::from_low_u64_be(1)).unwrap(),
	})
}

/// What the receipt of a transaction that went through records.
pub fn outcome(cumulative_gas: u64, logs: Vec<Log>) -> EIP658ReceiptData {
	EIP658ReceiptData { status_code: 1, used_gas: cumulative_gas.into(), logs_bloom: Bloom::zero(), logs }
}

pub fn log(address: H160, topic: H256) -> Log {
	Log { address, topics: vec![topic], data: Vec::new() }
}

impl Node for FakeNode {
	fn best_number(&self) -> u32 {
		self.best
	}

	fn finalized_number(&self) -> u32 {
		self.finalized.load(Ordering::Relaxed)
	}

	fn hash(&self, number: u32) -> Option<H256> {
		self.headers.get(&number).map(Header::hash)
	}

	fn header(&self, hash: H256) -> Option<Header> {
		self.headers.get(self.numbers.get(&hash)?).cloned()
	}

	fn api_version(&self, _at: H256, _api: &[u8; 8]) -> Option<u32> {
		self.api_version
	}

	fn call(
		&self,
		at: H256,
		function: &'static str,
		arguments: Vec<u8>,
		changes: &RefCell<Changes>,
		metered: bool,
	) -> Result<Vec<u8>, ApiError> {
		if let Some((key, value)) = self.writes.get(function) {
			changes.borrow_mut().set_storage(key.clone(), Some(value.clone()));
		}
		let answer = self
			.answers
			.get(&(at, function.to_owned(), arguments.clone()))
			.cloned()
			.or_else(|| self.responders.get(&(at, function)).map(|responder| responder(&arguments)));
		self.called.lock().unwrap().push(Called { function, arguments, changes: changes.borrow().clone(), metered });
		answer.ok_or_else(|| ApiError::Application(format!("no answer for {function}").into()))
	}

	fn storage(&self, at: H256, key: &[u8]) -> Option<Vec<u8>> {
		self.storage.get(&(at, key.to_vec())).cloned()
	}

	fn aux(&self, key: &[u8]) -> Option<Vec<u8>> {
		self.aux.lock().unwrap().get(key).cloned()
	}

	fn insert_aux(&self, entries: &[(Vec<u8>, Vec<u8>)]) -> sp_blockchain::Result<()> {
		self.aux_writes.fetch_add(1, Ordering::Relaxed);
		self.aux.lock().unwrap().extend(entries.iter().cloned());
		Ok(())
	}
}
