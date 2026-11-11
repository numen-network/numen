//! Shapes Ethereum blocks, transactions, receipts and logs the way Frontier's
//! RPC did.

use std::collections::BTreeMap;

use ethereum::{EIP658ReceiptData, ReceiptV4, TransactionV3};
use ethereum_types::U64;
use fc_rpc_core::types::{
	Block, BlockTransactions, BuildFrom, Bytes, Filter, FilteredParams, Header, Log, Receipt, Rich, RichBlock,
	Transaction,
};
use sp_core::U256;

use super::runtime::{BlockData, TransactionStatus};

pub fn block(data: &BlockData, full: bool) -> RichBlock {
	let header = &data.block.header;
	Rich {
		inner: Block {
			header: Header {
				hash: Some(header.hash()),
				parent_hash: header.parent_hash,
				uncles_hash: header.ommers_hash,
				author: header.beneficiary,
				miner: Some(header.beneficiary),
				state_root: header.state_root,
				transactions_root: header.transactions_root,
				receipts_root: header.receipts_root,
				number: Some(header.number),
				gas_used: header.gas_used,
				gas_limit: header.gas_limit,
				extra_data: Bytes(header.extra_data.clone()),
				logs_bloom: header.logs_bloom,
				timestamp: U256::from(header.timestamp / 1000),
				difficulty: header.difficulty,
				nonce: Some(header.nonce),
				size: Some(U256::from(rlp::encode(header).len())),
			},
			total_difficulty: Some(U256::zero()),
			uncles: Vec::new(),
			transactions: transactions(data, full),
			size: Some(U256::from(rlp::encode(&data.block).len())),
			base_fee_per_gas: Some(data.base_fee),
		},
		extra_info: BTreeMap::new(),
	}
}

fn transactions(data: &BlockData, full: bool) -> BlockTransactions {
	let signed = &data.block.transactions;
	if !full {
		return BlockTransactions::Hashes(signed.iter().map(TransactionV3::hash).collect());
	}
	BlockTransactions::Full(
		signed.iter().zip(&data.statuses).map(|(signed, status)| transaction(data, signed, status)).collect(),
	)
}

pub fn transaction(data: &BlockData, signed: &TransactionV3, status: &TransactionStatus) -> Transaction {
	let mut transaction = Transaction::build_from(status.from, signed);
	if let TransactionV3::EIP1559(_) = signed {
		let tip = transaction.max_priority_fee_per_gas.unwrap_or_default();
		let cap = transaction.max_fee_per_gas.unwrap_or_default();
		transaction.gas_price = Some(paid(data.base_fee, tip, cap));
	}
	transaction.block_hash = Some(data.block.header.hash());
	transaction.block_number = Some(data.block.header.number);
	transaction.transaction_index = Some(U256::from(status.transaction_index));
	transaction.creates = status.contract_address;
	transaction
}

/// The gas price a dynamic fee transaction pays at this base fee.
fn paid(base_fee: U256, tip: U256, cap: U256) -> U256 {
	base_fee.checked_add(tip).unwrap_or_else(U256::max_value).min(cap)
}

fn outcome(receipt: &ReceiptV4) -> &EIP658ReceiptData {
	match receipt {
		ReceiptV4::Legacy(outcome) |
		ReceiptV4::EIP2930(outcome) |
		ReceiptV4::EIP1559(outcome) |
		ReceiptV4::EIP7702(outcome) => outcome,
	}
}

/// The receipt of the transaction at `index`. Dynamic fee transactions are
/// priced off `parent_base_fee`, the base fee of the block before.
pub fn receipt(data: &BlockData, index: usize, parent_base_fee: U256) -> Option<Receipt> {
	let (signed, status) = (data.block.transactions.get(index)?, data.statuses.get(index)?);
	let (receipt, earlier) = (data.receipts.get(index)?, &data.receipts[..index]);
	let block_hash = data.block.header.hash();
	let gas_before = earlier.last().map_or_else(U256::zero, |receipt| outcome(receipt).used_gas);
	let logs_before: usize = earlier.iter().map(|receipt| outcome(receipt).logs.len()).sum();
	let logs = outcome(receipt).logs.iter().enumerate().map(|(position, log)| Log {
		address: log.address,
		topics: log.topics.clone(),
		data: Bytes(log.data.clone()),
		block_hash: Some(block_hash),
		block_number: Some(data.block.header.number),
		transaction_hash: Some(status.transaction_hash),
		transaction_index: Some(status.transaction_index.into()),
		log_index: Some(U256::from(logs_before + position)),
		transaction_log_index: Some(U256::from(position)),
		removed: false,
	});
	Some(Receipt {
		transaction_hash: Some(status.transaction_hash),
		transaction_index: Some(status.transaction_index.into()),
		block_hash: Some(block_hash),
		from: Some(status.from),
		to: status.to,
		block_number: Some(data.block.header.number),
		cumulative_gas_used: outcome(receipt).used_gas,
		gas_used: Some(outcome(receipt).used_gas.saturating_sub(gas_before)),
		contract_address: status.contract_address,
		logs: logs.collect(),
		state_root: None,
		logs_bloom: outcome(receipt).logs_bloom,
		status_code: Some(U64::from(outcome(receipt).status_code)),
		effective_gas_price: match signed {
			TransactionV3::Legacy(signed) => signed.gas_price,
			TransactionV3::EIP2930(signed) => signed.gas_price,
			TransactionV3::EIP1559(signed) =>
				paid(parent_base_fee, signed.max_priority_fee_per_gas, signed.max_fee_per_gas),
			TransactionV3::EIP7702(signed) =>
				paid(parent_base_fee, signed.max_priority_fee_per_gas, signed.max_fee_per_gas),
		},
		transaction_type: match receipt {
			ReceiptV4::Legacy(_) => U256::from(0),
			ReceiptV4::EIP2930(_) => U256::from(1),
			ReceiptV4::EIP1559(_) => U256::from(2),
			ReceiptV4::EIP7702(_) => U256::from(4),
		},
	})
}

/// The logs of the block that pass `filter`.
pub fn logs(filter: &Filter, data: &BlockData) -> Vec<Log> {
	let params = FilteredParams::new(filter.clone());
	let block_hash = data.block.header.hash();
	let in_block = data.statuses.iter().flat_map(|status| {
		status.logs.iter().enumerate().map(move |(position, log)| (status, position, log))
	});
	in_block
		.enumerate()
		.map(|(index, (status, position, log))| Log {
			address: log.address,
			topics: log.topics.clone(),
			data: Bytes(log.data.clone()),
			block_hash: Some(block_hash),
			block_number: Some(data.block.header.number),
			transaction_hash: Some(status.transaction_hash),
			transaction_index: Some(U256::from(status.transaction_index)),
			log_index: Some(U256::from(index)),
			transaction_log_index: Some(U256::from(position)),
			removed: false,
		})
		.filter(|log| passes(&params, log))
		.collect()
}

fn passes(params: &FilteredParams, log: &Log) -> bool {
	let filter = &params.filter;
	log.block_hash.is_none_or(|hash| params.filter_block_hash(hash)) &&
		(filter.topics().is_empty() || params.filter_topics(&log.topics)) &&
		filter.address.as_ref().is_none_or(|_| params.filter_address(&log.address))
}
