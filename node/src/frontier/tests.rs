use std::sync::Arc;

use codec::Encode;
use ethereum::TransactionV3;
use serde_json::{json, Value};
use sp_core::{H160, H256, U256};

use super::{fake::*, Frontier, Node, MAX_BLOCK_RANGE, MAX_PAST_LOGS, RUNTIME_API_VERSION};
use crate::eth_forward::History;

const TOKEN: H160 = H160::repeat_byte(0x70);
const OTHER_TOKEN: H160 = H160::repeat_byte(0x71);
const TRANSFER: H256 = H256::repeat_byte(0x01);
const APPROVAL: H256 = H256::repeat_byte(0x02);

fn history(node: FakeNode) -> Frontier<FakeNode> {
	Frontier::new(Arc::new(node)).expect("the fake chain starts with Frontier")
}

fn ask(history: &Frontier<FakeNode>, method: &str, params: Value) -> Result<Value, (i32, String)> {
	let answer = history.call(method, params.as_array().expect("parameters are a list"));
	answer.map_err(|error| (error.code(), error.message().to_owned()))
}

fn quantity(number: impl Into<U256>) -> Value {
	json!(format!("{:#x}", number.into()))
}

/// A chain whose block 2 holds two transactions. The first leaves two logs
/// and the second one.
fn chain_with_logs() -> (FakeNode, Vec<TransactionV3>) {
	let mut node = FakeNode::new();
	node.hold(1, U256::from(100), Vec::new());
	let first = (transaction(0, 10, 1_000), outcome(21_000, vec![log(TOKEN, TRANSFER), log(TOKEN, APPROVAL)]));
	let second = (transaction(1, 10, 1_000), outcome(50_000, vec![log(OTHER_TOKEN, TRANSFER)]));
	let block = node.hold(2, U256::from(300), vec![first, second]);
	node.finalize(2);
	(node, block.transactions)
}

/// The fixture holds blocks, runtime answers and Frontier's answers, all
/// recorded off mainnet. Its blocks are far apart, so they are indexed one by
/// one.
#[test]
fn mainnet_requests_get_the_answers_frontier_gave() {
	let (node, blocks, recorded) = FakeNode::mainnet();
	node.finalize(node.best);
	let history = history(node);
	for number in blocks {
		let (entries, _) = history.index.entries(number, number);
		history.node.insert_aux(&entries).unwrap();
	}

	for request in recorded {
		let (method, params) = (request["method"].as_str().unwrap(), request["params"].as_array().unwrap());
		let expected = match request.get("error") {
			Some(error) => Err(error.clone()),
			None => Ok(request["result"].clone()),
		};

		let answer = history.call(method, params).map_err(|error| serde_json::to_value(error).unwrap());

		assert_eq!(answer, expected, "{method} {params:?}");
	}
}

#[test]
fn a_receipt_numbers_its_logs_after_those_of_the_transactions_before() {
	let (node, transactions) = chain_with_logs();
	let history = history(node);

	let receipt = ask(&history, "eth_getTransactionReceipt", json!([transactions[1].hash()])).unwrap();

	assert_eq!(receipt["logs"][0]["logIndex"], "0x2");
	assert_eq!(receipt["logs"][0]["transactionLogIndex"], "0x0");
}

#[test]
fn a_receipt_reports_the_gas_its_transaction_added_to_the_block() {
	let (node, transactions) = chain_with_logs();
	let history = history(node);

	let receipt = ask(&history, "eth_getTransactionReceipt", json!([transactions[1].hash()])).unwrap();

	assert_eq!(receipt["cumulativeGasUsed"], quantity(50_000));
	assert_eq!(receipt["gasUsed"], quantity(50_000 - 21_000));
}

#[test]
fn the_logs_of_a_block_are_numbered_across_its_transactions() {
	let (node, _) = chain_with_logs();
	let history = history(node);

	let logs = ask(&history, "eth_getLogs", json!([{ "fromBlock": "0x2", "toBlock": "0x2" }])).unwrap();

	let numbered: Vec<_> = logs
		.as_array()
		.unwrap()
		.iter()
		.map(|log| {
			let field = |name| log[name].as_str().unwrap();
			(field("transactionIndex"), field("transactionLogIndex"), field("logIndex"))
		})
		.collect();
	assert_eq!(numbered, [("0x0", "0x0", "0x0"), ("0x0", "0x1", "0x1"), ("0x1", "0x0", "0x2")]);
}

#[test]
fn a_log_filter_keeps_the_logs_that_match_address_and_topic() {
	let (node, _) = chain_with_logs();
	let history = history(node);
	let matching = |filter: Value| {
		let logs = ask(&history, "eth_getLogs", json!([filter])).unwrap();
		logs.as_array().unwrap().iter().map(|log| log["logIndex"].clone()).collect::<Vec<_>>()
	};

	assert_eq!(matching(json!({ "fromBlock": "0x2", "toBlock": "0x2", "address": TOKEN })), ["0x0", "0x1"]);
	assert_eq!(matching(json!({ "fromBlock": "0x2", "toBlock": "0x2", "topics": [TRANSFER] })), ["0x0", "0x2"]);
	assert_eq!(
		matching(json!({ "fromBlock": "0x2", "toBlock": "0x2", "address": OTHER_TOKEN, "topics": [APPROVAL] })),
		[] as [Value; 0],
	);
}

/// The base fee is 100 in block 1 and 300 in block 2, where the transaction
/// is. Its tip is 10.
#[test]
fn a_dynamic_fee_receipt_is_priced_off_the_base_fee_of_the_block_before() {
	let (node, transactions) = chain_with_logs();
	let history = history(node);

	let receipt = ask(&history, "eth_getTransactionReceipt", json!([transactions[0].hash()])).unwrap();
	let transaction = ask(&history, "eth_getTransactionByHash", json!([transactions[0].hash()])).unwrap();

	assert_eq!(receipt["effectiveGasPrice"], quantity(100 + 10));
	assert_eq!(transaction["gasPrice"], quantity(300 + 10));
}

/// Base fee 100 plus a tip of 10 comes to 110.
#[test]
fn a_dynamic_fee_transaction_pays_no_more_than_its_fee_cap() {
	let mut node = FakeNode::new();
	node.hold(1, U256::from(100), Vec::new());
	let capped = |cap| (transaction(cap, 10, cap), outcome(21_000, Vec::new()));
	let block = node.hold(2, U256::from(100), vec![capped(109), capped(110), capped(111)]);
	node.finalize(2);
	let history = history(node);

	let paid: Vec<_> = block
		.transactions
		.iter()
		.map(|signed| ask(&history, "eth_getTransactionReceipt", json!([signed.hash()])).unwrap()["effectiveGasPrice"].clone())
		.collect();

	assert_eq!(paid, [quantity(109), quantity(110), quantity(110)]);
}

#[test]
fn a_log_range_may_span_the_maximum_and_no_more() {
	let mut node = FakeNode::new();
	node.best = MAX_BLOCK_RANGE * 2;
	let history = history(node);
	let range = |to: u32| json!([{ "fromBlock": "0x0", "toBlock": quantity(to) }]);

	assert_eq!(ask(&history, "eth_getLogs", range(MAX_BLOCK_RANGE)), Ok(json!([])));
	assert_eq!(
		ask(&history, "eth_getLogs", range(MAX_BLOCK_RANGE + 1)),
		Err((-32603, format!("block range is too wide (maximum {MAX_BLOCK_RANGE})"))),
	);
}

#[test]
fn a_log_range_ends_at_the_best_block() {
	let (node, _) = chain_with_logs();
	let history = history(node);

	let logs = ask(&history, "eth_getLogs", json!([{ "fromBlock": "0x2", "toBlock": "0xffffff" }])).unwrap();
	let none = ask(&history, "eth_getLogs", json!([{ "fromBlock": "0x3", "toBlock": "0xffffff" }])).unwrap();

	assert_eq!(logs.as_array().unwrap().len(), 3);
	assert_eq!(none, json!([]));
}

#[test]
fn a_log_answer_may_hold_the_maximum_and_no_more() {
	let mut node = FakeNode::new();
	let full = vec![log(TOKEN, TRANSFER); MAX_PAST_LOGS];
	node.hold(1, U256::one(), vec![(transaction(0, 0, 1), outcome(21_000, full))]);
	node.hold(2, U256::one(), vec![(transaction(1, 0, 1), outcome(21_000, vec![log(TOKEN, TRANSFER)]))]);
	let history = history(node);
	let range = |to: &str| json!([{ "fromBlock": "0x1", "toBlock": to }]);

	let at_the_limit = ask(&history, "eth_getLogs", range("0x1")).unwrap();
	let over_it = ask(&history, "eth_getLogs", range("0x2"));

	assert_eq!(at_the_limit.as_array().unwrap().len(), MAX_PAST_LOGS);
	assert_eq!(over_it, Err((-32603, format!("query returned more than {MAX_PAST_LOGS} results"))));
}

#[test]
fn logs_of_a_block_hash_nobody_has_are_refused() {
	let (node, _) = chain_with_logs();
	let history = history(node);

	let answer = ask(&history, "eth_getLogs", json!([{ "blockHash": H256::repeat_byte(0x11) }]));

	assert_eq!(answer, Err((-32000, "unknown block".into())));
}

/// The account holds 7 with a nonce of 3 in block 1, the best one.
#[test]
fn account_reads_stop_at_the_best_block() {
	let mut node = FakeNode::new();
	node.hold(1, U256::one(), Vec::new());
	node.answer(1, ACCOUNT_BASIC, SENDER.encode(), (U256::from(7), U256::from(3)).encode());
	let history = history(node);

	assert_eq!(ask(&history, "eth_getBalance", json!([SENDER, "0x1"])), Ok(json!("0x7")));
	assert_eq!(ask(&history, "eth_getTransactionCount", json!([SENDER, "0x1"])), Ok(json!("0x3")));
	for method in ["eth_getBalance", "eth_getTransactionCount", "eth_getCode"] {
		assert_eq!(
			ask(&history, method, json!([SENDER, "0x2"])),
			Err((-32603, "Expect block number from id: Number(2)".into())),
			"{method}",
		);
	}
}

#[test]
fn an_account_read_without_a_block_looks_at_the_best_one() {
	let mut node = FakeNode::new();
	node.hold(1, U256::one(), Vec::new());
	node.answer(1, ACCOUNT_BASIC, SENDER.encode(), (U256::from(7), U256::from(3)).encode());
	let history = history(node);

	assert_eq!(ask(&history, "eth_getBalance", json!([SENDER])), Ok(json!("0x7")));
}

#[test]
fn a_call_on_a_block_past_the_best_one_finds_no_header() {
	let mut node = FakeNode::new();
	node.hold(1, U256::one(), Vec::new());
	let history = history(node);

	for method in ["eth_call", "eth_estimateGas"] {
		let answer = ask(&history, method, json!([{ "to": TOKEN }, "0x2"]));

		assert_eq!(answer, Err((-32000, "header not found".into())), "{method}");
	}
}

#[test]
fn an_account_at_a_block_hash_nobody_has_reads_as_empty() {
	let history = history(FakeNode::new());
	let nowhere = json!({ "blockHash": H256::repeat_byte(0x11) });

	assert_eq!(ask(&history, "eth_getBalance", json!([SENDER, nowhere])), Ok(json!("0x0")));
	assert_eq!(ask(&history, "eth_getTransactionCount", json!([SENDER, nowhere])), Ok(json!("0x0")));
	assert_eq!(ask(&history, "eth_getCode", json!([SENDER, nowhere])), Ok(json!("0x")));
	assert_eq!(ask(&history, "eth_getStorageAt", json!([SENDER, "0x0", nowhere])), Ok(json!(H256::zero())));
}

#[test]
fn a_block_nobody_has_is_null() {
	let history = history(FakeNode::new());
	let nowhere = H256::repeat_byte(0x11);

	assert_eq!(ask(&history, "eth_getBlockByNumber", json!(["0x1", false])), Ok(Value::Null));
	assert_eq!(ask(&history, "eth_getBlockByHash", json!([nowhere, false])), Ok(Value::Null));
	assert_eq!(ask(&history, "eth_getBlockReceipts", json!(["0x1"])), Ok(Value::Null));
	assert_eq!(ask(&history, "eth_getTransactionByHash", json!([nowhere])), Ok(Value::Null));
	assert_eq!(ask(&history, "eth_getTransactionReceipt", json!([nowhere])), Ok(Value::Null));
}

#[test]
fn a_missing_parameter_is_reported_as_invalid() {
	let history = history(FakeNode::new());

	let answer = history.call("eth_getBlockByNumber", &[json!("0x0")]).unwrap_err();

	assert_eq!((answer.code(), answer.message()), (-32602, "Invalid params"));
	assert_eq!(answer.data().map(|data| data.get()), Some("\"No more params\""));
}

#[test]
fn only_a_chain_frontier_started_has_a_history() {
	let with = |api_version| {
		let mut node = FakeNode::new();
		node.api_version = api_version;
		Frontier::new(Arc::new(node)).is_some()
	};

	assert!(with(Some(RUNTIME_API_VERSION)));
	assert!(!with(Some(RUNTIME_API_VERSION - 1)));
	assert!(!with(Some(RUNTIME_API_VERSION + 1)));
	assert!(!with(None));
}
