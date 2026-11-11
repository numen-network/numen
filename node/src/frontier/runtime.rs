//! The runtime API Frontier's RPC read through, called by name.

use std::cell::RefCell;

use codec::{Decode, Encode};
use ethereum::{BlockV3, Log, ReceiptV4};
use ethereum_types::Bloom;
use evm_core::ExitReason;
use sp_api::ApiError;
use sp_core::{H160, H256, U256};
use sp_runtime::DispatchError;

use super::node::{Changes, Node};

#[derive(Decode)]
pub struct TransactionStatus {
	pub transaction_hash: H256,
	pub transaction_index: u32,
	pub from: H160,
	pub to: Option<H160>,
	pub contract_address: Option<H160>,
	pub logs: Vec<Log>,
	_logs_bloom: Bloom,
}

#[derive(Decode)]
pub struct Account {
	pub balance: U256,
	pub nonce: U256,
}

#[derive(Decode)]
pub struct UsedGas {
	_standard: U256,
	pub effective: U256,
}

/// How a call or a contract creation ended.
#[derive(Decode)]
pub struct Execution<T> {
	pub exit_reason: ExitReason,
	pub value: T,
	pub used_gas: UsedGas,
	_weight_info: Option<[Option<u64>; 4]>,
	_logs: Vec<Log>,
}

/// The Ethereum block a substrate block holds.
pub struct BlockData {
	pub block: BlockV3,
	pub receipts: Vec<ReceiptV4>,
	pub statuses: Vec<TransactionStatus>,
	pub base_fee: U256,
}

fn decode<T: Decode>(function: &'static str, raw: Vec<u8>) -> Result<T, ApiError> {
	T::decode(&mut &raw[..]).map_err(|error| ApiError::FailedToDecodeReturnValue { function, error, raw })
}

fn call<T: Decode>(
	node: &impl Node,
	at: H256,
	function: &'static str,
	arguments: Vec<u8>,
) -> Result<T, ApiError> {
	decode(function, node.call(at, function, arguments, &RefCell::default(), false)?)
}

pub fn block_data(node: &impl Node, at: H256) -> Option<BlockData> {
	type All = (Option<BlockV3>, Option<Vec<ReceiptV4>>, Option<Vec<TransactionStatus>>);
	let (block, receipts, statuses): All =
		call(node, at, "EthereumRuntimeRPCApi_current_all", Vec::new()).ok()?;
	Some(BlockData {
		block: block?,
		receipts: receipts?,
		statuses: statuses?,
		base_fee: gas_price(node, at).unwrap_or_default(),
	})
}

pub fn current_block(node: &impl Node, at: H256) -> Result<Option<BlockV3>, ApiError> {
	call(node, at, "EthereumRuntimeRPCApi_current_block", Vec::new())
}

pub fn gas_price(node: &impl Node, at: H256) -> Result<U256, ApiError> {
	call(node, at, "EthereumRuntimeRPCApi_gas_price", Vec::new())
}

pub fn account_basic(node: &impl Node, at: H256, address: H160) -> Result<Account, ApiError> {
	call(node, at, "EthereumRuntimeRPCApi_account_basic", address.encode())
}

pub fn account_code_at(node: &impl Node, at: H256, address: H160) -> Result<Vec<u8>, ApiError> {
	account_code_on(node, at, address, &RefCell::default())
}

/// The code of `address` in the state of block `at` with `changes` on top.
pub fn account_code_on(
	node: &impl Node,
	at: H256,
	address: H160,
	changes: &RefCell<Changes>,
) -> Result<Vec<u8>, ApiError> {
	let function = "EthereumRuntimeRPCApi_account_code_at";
	decode(function, node.call(at, function, address.encode(), changes, false)?)
}

pub fn storage_at(node: &impl Node, at: H256, address: H160, index: U256) -> Result<H256, ApiError> {
	call(node, at, "EthereumRuntimeRPCApi_storage_at", (address, index).encode())
}

pub fn supports_gas_limit_multiplier(node: &impl Node, at: H256) -> bool {
	call::<()>(node, at, "EthereumRuntimeRPCApi_gas_limit_multiplier_support", Vec::new()).is_ok()
}

/// Runs `function`, the runtime's `call` or `create`, on top of `changes`.
pub fn execute<T: Decode>(
	node: &impl Node,
	at: H256,
	function: &'static str,
	arguments: Vec<u8>,
	changes: &RefCell<Changes>,
) -> Result<Result<Execution<T>, DispatchError>, ApiError> {
	decode(function, node.call(at, function, arguments, changes, true)?)
}
