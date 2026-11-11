//! Ethereum JSON-RPC answers for the blocks Frontier built.
//!
//! The node does not link Frontier. The runtime that built each of those
//! blocks still sits in its parent's state. The node calls that runtime's
//! Ethereum API by name on the block's own state, and shapes the answers the
//! way Frontier's RPC did.

mod execute;
#[cfg(test)]
mod fake;
mod index;
mod node;
mod runtime;
mod shape;
#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Instant};

use fc_rpc_core::types::{
	BlockNumberOrHash, Bytes, Filter, Index as Position, Log, Receipt, RichBlock, Transaction,
};
use jsonrpsee::{
	core::RpcResult,
	types::{
		error::{CALL_EXECUTION_FAILED_CODE, INTERNAL_ERROR_CODE, METHOD_NOT_FOUND_CODE},
		ErrorObjectOwned, Params,
	},
};
use serde::Serialize;
use serde_json::Value;
use sp_core::{H160, H256, U256};
use sp_runtime::{generic::BlockId, traits::UniqueSaturatedInto};

use numen_runtime::opaque::Block;

use crate::eth_forward::History;
pub use index::FRONTIER_ENGINE_ID;
use index::Index;
pub use node::{ClientNode, Node};
use runtime::BlockData;

/// The version of `EthereumRuntimeRPCApi` whose calls this module makes.
const RUNTIME_API_VERSION: u32 = 7;

/// Blocks one `eth_getLogs` range may span.
const MAX_BLOCK_RANGE: u32 = 1024;

/// Logs one `eth_getLogs` answer may hold.
const MAX_PAST_LOGS: usize = 10_000;

/// Seconds one `eth_getLogs` range may take.
const MAX_LOGS_SECONDS: u64 = 10;

fn error(code: i32, message: impl ToString, data: Option<&[u8]>) -> ErrorObjectOwned {
	ErrorObjectOwned::owned(code, message.to_string(), data.map(|data| sp_core::bytes::to_hex(data, false)))
}

fn internal_err(message: impl ToString) -> ErrorObjectOwned {
	error(INTERNAL_ERROR_CODE, message, None)
}

fn internal_err_with_data(message: impl ToString, data: &[u8]) -> ErrorObjectOwned {
	error(INTERNAL_ERROR_CODE, message, Some(data))
}

fn json(answer: RpcResult<impl Serialize>) -> Result<Value, ErrorObjectOwned> {
	serde_json::to_value(answer?).map_err(internal_err)
}

/// The Frontier part of the chain, read from the node's own state.
pub struct Frontier<N> {
	node: Arc<N>,
	index: Index<N>,
}

impl<N: Node> Frontier<N> {
	/// None on a chain whose first blocks Frontier did not build.
	pub fn new(node: Arc<N>) -> Option<Self> {
		let api = sp_crypto_hashing::blake2_64(b"EthereumRuntimeRPCApi");
		(node.api_version(node.hash(0)?, &api)? == RUNTIME_API_VERSION)
			.then(|| Self { index: Index::new(node.clone()), node })
	}

	/// Indexes the finalized Frontier blocks not indexed yet.
	pub fn catch_up(&self) {
		self.index.catch_up();
	}

	/// The block a request names. None for an Ethereum hash no block has and
	/// for the pending block.
	fn locate(&self, block: BlockNumberOrHash) -> Option<BlockId<Block>> {
		Some(match block {
			BlockNumberOrHash::Hash { hash, .. } => BlockId::Hash(self.node.hash(self.index.block(hash)?)?),
			BlockNumberOrHash::Num(number) => BlockId::Number(number.unique_saturated_into()),
			BlockNumberOrHash::Latest => BlockId::Number(self.node.best_number()),
			BlockNumberOrHash::Earliest => BlockId::Number(0),
			BlockNumberOrHash::Safe | BlockNumberOrHash::Finalized => BlockId::Number(self.node.finalized_number()),
			BlockNumberOrHash::Pending => return None,
		})
	}

	fn hash(&self, id: &BlockId<Block>) -> Option<H256> {
		match id {
			BlockId::Hash(hash) => Some(*hash),
			BlockId::Number(number) => self.node.hash(*number),
		}
	}

	fn data(&self, block: BlockNumberOrHash) -> Option<BlockData> {
		runtime::block_data(self.node.as_ref(), self.hash(&self.locate(block)?)?)
	}

	fn data_at(&self, number: u32) -> Option<BlockData> {
		runtime::block_data(self.node.as_ref(), self.node.hash(number)?)
	}

	/// The block a state read looks at. None when `locate` finds none.
	fn read_at(&self, block: Option<BlockNumberOrHash>) -> RpcResult<Option<H256>> {
		let Some(id) = self.locate(block.unwrap_or_default()) else { return Ok(None) };
		self.hash(&id).map(Some).ok_or_else(|| internal_err(format!("Expect block number from id: {id}")))
	}

	/// The block a call runs on.
	fn run_at(&self, block: Option<BlockNumberOrHash>) -> RpcResult<H256> {
		self.locate(block.unwrap_or_default())
			.and_then(|id| self.hash(&id))
			.ok_or_else(|| error(CALL_EXECUTION_FAILED_CODE, "header not found", None))
	}

	fn block(&self, block: BlockNumberOrHash, full: bool) -> Option<RichBlock> {
		self.data(block).map(|data| shape::block(&data, full))
	}

	fn transaction_count(&self, block: BlockNumberOrHash) -> Option<U256> {
		self.data(block).map(|data| U256::from(data.block.transactions.len()))
	}

	/// The base fee of the block before. Receipts price dynamic fee
	/// transactions off it.
	fn parent_base_fee(&self, data: &BlockData) -> U256 {
		let parent: u32 = data.block.header.number.saturating_sub(U256::one()).unique_saturated_into();
		let fee = self.node.hash(parent).map(|parent| runtime::gas_price(self.node.as_ref(), parent));
		fee.and_then(Result::ok).unwrap_or_default()
	}

	fn receipts(&self, block: BlockNumberOrHash) -> Option<Vec<Receipt>> {
		let data = self.data(block)?;
		let parent_base_fee = self.parent_base_fee(&data);
		Some((0..data.statuses.len()).filter_map(|index| shape::receipt(&data, index, parent_base_fee)).collect())
	}

	fn transaction(&self, hash: H256) -> Option<Transaction> {
		let (number, position) = self.index.transaction(hash)?;
		let data = self.data_at(number)?;
		let position = position as usize;
		Some(shape::transaction(&data, data.block.transactions.get(position)?, data.statuses.get(position)?))
	}

	fn transaction_at(&self, block: BlockNumberOrHash, position: Position) -> RpcResult<Option<Transaction>> {
		let Some(data) = self.data(block) else { return Ok(None) };
		let position = position.value();
		match (data.block.transactions.get(position), data.statuses.get(position)) {
			(Some(signed), Some(status)) => Ok(Some(shape::transaction(&data, signed, status))),
			_ => Err(internal_err(format!("{position:?} is out of bounds"))),
		}
	}

	fn receipt(&self, hash: H256) -> Option<Receipt> {
		let (number, position) = self.index.transaction(hash)?;
		let data = self.data_at(number)?;
		shape::receipt(&data, position as usize, self.parent_base_fee(&data))
	}

	fn balance(&self, address: H160, block: Option<BlockNumberOrHash>) -> RpcResult<U256> {
		let Some(at) = self.read_at(block)? else { return Ok(U256::zero()) };
		let account = runtime::account_basic(self.node.as_ref(), at, address);
		Ok(account.map_err(|error| internal_err(format!("Fetch account balances failed: {error:?}")))?.balance)
	}

	fn nonce(&self, address: H160, block: Option<BlockNumberOrHash>) -> RpcResult<U256> {
		let Some(at) = self.read_at(block)? else { return Ok(U256::zero()) };
		let account = runtime::account_basic(self.node.as_ref(), at, address);
		Ok(account.map_err(|error| internal_err(format!("Fetch account nonce failed: {error}")))?.nonce)
	}

	fn storage(&self, address: H160, slot: U256, block: Option<BlockNumberOrHash>) -> RpcResult<H256> {
		let Some(at) = self.read_at(block)? else { return Ok(H256::default()) };
		Ok(runtime::storage_at(self.node.as_ref(), at, address, slot).unwrap_or_default())
	}

	fn code(&self, address: H160, block: Option<BlockNumberOrHash>) -> RpcResult<Bytes> {
		let Some(at) = self.read_at(block)? else { return Ok(Bytes(Vec::new())) };
		Ok(Bytes(runtime::account_code_at(self.node.as_ref(), at, address).unwrap_or_default()))
	}

	fn logs(&self, filter: Filter) -> RpcResult<Vec<Log>> {
		if let Some(hash) = filter.block_hash {
			let number = self.index.block(hash).ok_or_else(|| error(-32000, "unknown block", None))?;
			return Ok(self.data_at(number).map_or_else(Vec::new, |data| shape::logs(&filter, &data)));
		}
		let latest = self.node.best_number();
		let bound = |block: Option<BlockNumberOrHash>| -> u32 {
			block.and_then(|block| block.to_min_block_num()).map_or(latest, UniqueSaturatedInto::unique_saturated_into)
		};
		let (from, to) = (bound(filter.from_block), bound(filter.to_block).min(latest));
		if to.saturating_sub(from) > MAX_BLOCK_RANGE {
			return Err(internal_err(format!("block range is too wide (maximum {MAX_BLOCK_RANGE})")));
		}
		let started = Instant::now();
		let mut logs = Vec::new();
		for number in from..=to {
			if self.index.hashes(number).is_some_and(|hashes| !hashes.transaction_hashes.is_empty()) {
				logs.extend(self.data_at(number).iter().flat_map(|data| shape::logs(&filter, data)));
			}
			if logs.len() > MAX_PAST_LOGS {
				return Err(internal_err(format!("query returned more than {MAX_PAST_LOGS} results")));
			}
			if started.elapsed().as_secs() > MAX_LOGS_SECONDS {
				return Err(internal_err(format!("query timeout of {MAX_LOGS_SECONDS} seconds exceeded")));
			}
		}
		Ok(logs)
	}
}

impl<N: Node> History for Frontier<N> {
	fn call(&self, method: &str, params: &[Value]) -> Result<Value, ErrorObjectOwned> {
		let params = serde_json::to_string(params).map_err(internal_err)?;
		let params = Params::new(Some(&params));
		let mut params = params.sequence();
		let by_hash = |hash| BlockNumberOrHash::Hash { hash, require_canonical: false };
		let node = self.node.as_ref();
		match method {
			"eth_getBlockByHash" => json(Ok(self.block(by_hash(params.next()?), params.next()?))),
			"eth_getBlockByNumber" => json(Ok(self.block(params.next()?, params.next()?))),
			"eth_getBlockTransactionCountByHash" => json(Ok(self.transaction_count(by_hash(params.next()?)))),
			"eth_getBlockTransactionCountByNumber" => json(Ok(self.transaction_count(params.next()?))),
			"eth_getBlockReceipts" => json(Ok(self.receipts(params.next()?))),
			"eth_getTransactionByHash" => json(Ok(self.transaction(params.next()?))),
			"eth_getTransactionByBlockHashAndIndex" =>
				json(self.transaction_at(by_hash(params.next()?), params.next()?)),
			"eth_getTransactionByBlockNumberAndIndex" => json(self.transaction_at(params.next()?, params.next()?)),
			"eth_getTransactionReceipt" => json(Ok(self.receipt(params.next()?))),
			"eth_getBalance" => json(self.balance(params.next()?, params.optional_next()?)),
			"eth_getStorageAt" => json(self.storage(params.next()?, params.next()?, params.optional_next()?)),
			"eth_getTransactionCount" => json(self.nonce(params.next()?, params.optional_next()?)),
			"eth_getCode" => json(self.code(params.next()?, params.optional_next()?)),
			"eth_call" => {
				let (request, block) = (params.next()?, params.optional_next()?);
				let overrides = params.optional_next()?;
				json(execute::call(node, self.run_at(block), request, overrides))
			},
			"eth_estimateGas" => {
				let (request, block) = (params.next()?, params.optional_next()?);
				json(execute::estimate_gas(node, self.run_at(block)?, request))
			},
			"eth_getLogs" => json(self.logs(params.next()?)),
			_ => Err(error(METHOD_NOT_FOUND_CODE, "Method not found", None)),
		}
	}
}
