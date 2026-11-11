//! `eth_call` and `eth_estimateGas` on a block Frontier built.

use std::{cell::RefCell, collections::BTreeMap};

use codec::Encode;
use ethereum::AuthorizationListItem;
use evm_core::{ExitError, ExitReason};
use fc_rpc_core::types::{Bytes, CallStateOverride, TransactionRequest};
use jsonrpsee::{core::RpcResult, types::ErrorObjectOwned};
use sp_api::ApiError;
use sp_core::{H160, H256, U256};
use sp_crypto_hashing::{blake2_128, blake2_256, twox_128};
use sp_runtime::{DispatchError, SaturatedConversion};

use super::{
	internal_err, internal_err_with_data,
	node::{Changes, Node},
	runtime::{self, Execution},
};

/// How many block gas limits a call may use.
const GAS_LIMIT_MULTIPLIER: u64 = 10;

const MIN_GAS_PER_TX: U256 = U256([21_000, 0, 0, 0]);

type Overrides = Option<BTreeMap<H160, CallStateOverride>>;

/// Storage slots the runtime puts in place of an account's whole storage.
type StatePayload = Option<Vec<(H160, Vec<(H256, H256)>)>>;

struct Fees {
	max_fee_per_gas: Option<U256>,
	max_priority_fee_per_gas: Option<U256>,
	cap: U256,
}

fn fees(request: &TransactionRequest) -> RpcResult<Fees> {
	match (request.gas_price, request.max_fee_per_gas, request.max_priority_fee_per_gas) {
		(Some(_), Some(_), Some(_)) =>
			Err(internal_err("both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified")),
		(Some(gas_price), None, None) =>
			Ok(Fees { max_fee_per_gas: None, max_priority_fee_per_gas: None, cap: gas_price }),
		(None, Some(max_fee), Some(max_priority)) if max_priority > max_fee => Err(internal_err(
			"Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`",
		)),
		(None, Some(max_fee), Some(max_priority)) => Ok(Fees {
			max_fee_per_gas: Some(max_fee),
			max_priority_fee_per_gas: Some(max_priority),
			cap: max_fee,
		}),
		_ => Ok(Fees { max_fee_per_gas: None, max_priority_fee_per_gas: Some(U256::zero()), cap: U256::zero() }),
	}
}

/// The gas limit a request runs with and the highest one a call may use.
fn gas_limits(node: &impl Node, at: H256, requested: Option<U256>) -> RpcResult<(U256, U256)> {
	let block = runtime::current_block(node, at)
		.map_err(runtime_error)?
		.ok_or_else(|| internal_err("block unavailable, cannot query gas limit"))?;
	let highest = block.header.gas_limit.saturating_mul(GAS_LIMIT_MULTIPLIER.into());
	match requested {
		Some(gas) if gas > highest => Err(internal_err(format!(
			"provided gas limit is too high (can be up to {GAS_LIMIT_MULTIPLIER}x the block gas limit)"
		))),
		Some(gas) => Ok((gas, highest)),
		None if runtime::supports_gas_limit_multiplier(node, at) => Ok((highest, highest)),
		None => Ok((block.header.gas_limit, highest)),
	}
}

fn runtime_error(error: ApiError) -> ErrorObjectOwned {
	internal_err(format!("runtime error: {error}"))
}

fn access_list(request: &TransactionRequest) -> Vec<(H160, Vec<H256>)> {
	let items = request.access_list.clone().unwrap_or_default();
	items.into_iter().map(|item| (item.address, item.storage_keys)).collect()
}

fn input(request: &TransactionRequest) -> Vec<u8> {
	request.data.clone().into_bytes().map(Bytes::into_vec).unwrap_or_default()
}

/// How the runtime ran a request.
struct Run {
	exit_reason: ExitReason,
	/// What a call returned. Empty for a contract creation.
	data: Vec<u8>,
	/// The contract a creation made.
	created: Option<H160>,
	used_gas: U256,
}

struct Call<'a> {
	request: &'a TransactionRequest,
	gas_limit: U256,
	fees: &'a Fees,
	nonce: Option<U256>,
	estimate: bool,
	authorization_list: Option<Vec<AuthorizationListItem>>,
	state: StatePayload,
	changes: &'a RefCell<Changes>,
}

fn run(node: &impl Node, at: H256, call: Call) -> RpcResult<Run> {
	let Call { request, gas_limit, fees, nonce, estimate, authorization_list, state, changes } = call;
	let from = request.from.unwrap_or_default();
	let value = request.value.unwrap_or_default();
	let gas_limit = request.gas.unwrap_or(gas_limit).min(gas_limit);
	let (max_fee, max_priority) = (fees.max_fee_per_gas, fees.max_priority_fee_per_gas);
	let access_list = Some(access_list(request));
	let Some(to) = request.to else {
		let arguments =
			(from, input(request), value, gas_limit, max_fee, max_priority, nonce, estimate, access_list, authorization_list);
		let created: Execution<H160> =
			settle(runtime::execute(node, at, "EthereumRuntimeRPCApi_create", arguments.encode(), changes))?;
		return Ok(Run {
			exit_reason: created.exit_reason,
			data: Vec::new(),
			created: Some(created.value),
			used_gas: created.used_gas.effective,
		});
	};
	let arguments = (
		from,
		to,
		input(request),
		value,
		gas_limit,
		max_fee,
		max_priority,
		nonce,
		estimate,
		access_list,
		authorization_list,
		state,
	);
	let called: Execution<Vec<u8>> =
		settle(runtime::execute(node, at, "EthereumRuntimeRPCApi_call", arguments.encode(), changes))?;
	Ok(Run { exit_reason: called.exit_reason, data: called.value, created: None, used_gas: called.used_gas.effective })
}

fn settle<T>(executed: Result<Result<Execution<T>, DispatchError>, ApiError>) -> RpcResult<Execution<T>> {
	executed.map_err(runtime_error)?.map_err(|error| internal_err(format!("execution fatal: {error:?}")))
}

/// `at` is checked after the request, as Frontier's RPC did.
pub fn call(
	node: &impl Node,
	at: RpcResult<H256>,
	request: TransactionRequest,
	overrides: Overrides,
) -> RpcResult<Bytes> {
	if let Some((address, _)) =
		overrides.iter().flatten().find(|(_, account)| account.state.is_some() && account.state_diff.is_some())
	{
		return Err(internal_err(format!(
			"both `state` and `stateDiff` specified for account {address:?}; they are mutually exclusive"
		)));
	}
	let fees = fees(&request)?;
	let at = at?;
	let (gas_limit, _) = gas_limits(node, at, request.gas)?;
	// A contract creation runs without the overrides and leaves its code in
	// the changes.
	let changes = RefCell::new(match request.to {
		Some(_) => changes(node, at, overrides.as_ref()),
		None => Changes::default(),
	});
	let ran = run(
		node,
		at,
		Call {
			request: &request,
			gas_limit,
			fees: &fees,
			nonce: request.nonce,
			estimate: false,
			authorization_list: match request.to {
				Some(_) => request.authorization_list.clone(),
				None => Some(request.authorization_list.clone().unwrap_or_default()),
			},
			state: state_payload(&overrides),
			changes: &changes,
		},
	)?;
	failure(&ran.exit_reason, &ran.data)?;
	match ran.created {
		Some(contract) =>
			runtime::account_code_on(node, at, contract, &changes).map(Bytes).map_err(runtime_error),
		None => Ok(Bytes(ran.data)),
	}
}

pub fn estimate_gas(node: &impl Node, at: H256, request: TransactionRequest) -> RpcResult<U256> {
	if let Some(to) = request.to
		&& request.data().is_none_or(|data| data.0.is_empty())
		&& runtime::account_code_at(node, at, to).map_err(runtime_error)?.is_empty()
	{
		return Ok(MIN_GAS_PER_TX);
	}
	let (requested, highest) = gas_limits(node, at, request.gas)?;
	let fees = fees(&request)?;
	let cap = requested.min(affordable(node, at, &request, fees.cap)?);
	let attempt = |gas_limit| {
		run(
			node,
			at,
			Call {
				request: &request,
				gas_limit,
				fees: &fees,
				nonce: None,
				estimate: true,
				authorization_list: request.authorization_list.clone(),
				state: None,
				changes: &RefCell::default(),
			},
		)
	};
	let ran = attempt(cap)?;
	match &ran.exit_reason {
		ExitReason::Succeed(_) => Ok(ran.used_gas),
		ExitReason::Error(ExitError::OutOfGas) => Err(exceeds_allowance(cap)),
		ExitReason::Revert(_) if request.gas.is_some() || request.gas_price.is_some() => {
			let unbounded = attempt(highest)?;
			if unbounded.exit_reason.is_succeed() {
				return Err(exceeds_allowance(cap));
			}
			failure(&unbounded.exit_reason, &unbounded.data)?;
			Ok(ran.used_gas)
		},
		reason => failure(reason, &ran.data).map(|()| ran.used_gas),
	}
}

fn exceeds_allowance(cap: U256) -> ErrorObjectOwned {
	internal_err(format!("gas required exceeds allowance {cap}"))
}

/// The gas the sender can pay for at the fee cap after sending the value.
fn affordable(node: &impl Node, at: H256, request: &TransactionRequest, fee_cap: U256) -> RpcResult<U256> {
	let Some(from) = request.from.filter(|_| !fee_cap.is_zero()) else { return Ok(U256::MAX) };
	let balance = runtime::account_basic(node, at, from).map_err(runtime_error)?.balance;
	let value = request.value.unwrap_or_default();
	if value > balance {
		return Err(internal_err("insufficient funds for transfer"));
	}
	Ok((balance - value) / fee_cap)
}

fn failure(reason: &ExitReason, data: &[u8]) -> RpcResult<()> {
	match reason {
		ExitReason::Succeed(_) => Ok(()),
		ExitReason::Error(ExitError::OutOfGas) => Err(internal_err("out of gas")),
		ExitReason::Error(error) => Err(internal_err_with_data(format!("evm error: {error:?}"), &[])),
		ExitReason::Revert(_) => Err(internal_err_with_data(revert_message(data), data)),
		ExitReason::Fatal(error) => Err(internal_err_with_data(format!("evm fatal: {error:?}"), &[])),
	}
}

/// The message of a revert. It ends with the reason when the data holds an
/// ABI encoded `Error(string)`.
fn revert_message(data: &[u8]) -> String {
	const LENGTH: usize = 36;
	const REASON: usize = 68;
	let message = "VM Exception while processing transaction: revert";
	if data.len() <= REASON {
		return message.into();
	}
	let end = REASON.saturating_add(U256::from_big_endian(&data[LENGTH..REASON]).saturated_into());
	match data.get(REASON..end).map(std::str::from_utf8) {
		Some(Ok(reason)) => format!("{message} {reason}"),
		_ => message.into(),
	}
}

fn state_payload(overrides: &Overrides) -> StatePayload {
	let slots = |account: &CallStateOverride| {
		Some(account.state.as_ref()?.iter().map(|(slot, value)| (*slot, *value)).collect())
	};
	let payload: Vec<_> =
		overrides.iter().flatten().filter_map(|(address, account)| Some((*address, slots(account)?))).collect();
	(!payload.is_empty()).then_some(payload)
}

/// The storage changes that put the balance, nonce, code and single slot
/// overrides in place.
fn changes(node: &impl Node, at: H256, overrides: Option<&BTreeMap<H160, CallStateOverride>>) -> Changes {
	let mut changes = Changes::default();
	for (address, account) in overrides.into_iter().flatten() {
		let system_account = system_account_key(address);
		if let Some(mut info) = node.storage(at, &system_account).filter(|info| info.len() >= 32) {
			if let Some(nonce) = account.nonce {
				info.splice(0..4, nonce.low_u32().encode());
			}
			if let Some(balance) = account.balance {
				info.splice(16..32, balance.low_u128().encode());
			}
			changes.set_storage(system_account, Some(info));
		}
		if let Some(code) = &account.code {
			changes.set_storage(evm_key(b"AccountCodes", address), Some(code.clone().into_vec().encode()));
		}
		let Some(slots) = account.state_diff.as_ref().filter(|_| account.state.is_none()) else { continue };
		for (slot, value) in slots {
			let account_storage = evm_key(b"AccountStorages", address);
			let key = [&account_storage[..], &blake2_128(slot.as_bytes()), slot.as_bytes()].concat();
			changes.set_storage(key, Some(value.as_bytes().to_vec()));
		}
	}
	changes
}

/// Where `System::Account` keeps the account pallet-evm mapped `address` to.
fn system_account_key(address: &H160) -> Vec<u8> {
	let account = blake2_256(&[b"evm:", address.as_bytes()].concat());
	[&twox_128(b"System")[..], &twox_128(b"Account"), &blake2_128(&account), &account].concat()
}

fn evm_key(map: &[u8], address: &H160) -> Vec<u8> {
	[&twox_128(b"EVM")[..], &twox_128(map), &blake2_128(address.as_bytes()), address.as_bytes()].concat()
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use codec::Decode;
	use evm_core::{ExitFatal, ExitRevert, ExitSucceed};
	use serde_json::{json, Value};

	use super::*;
	use crate::{
		eth_forward::History,
		frontier::{fake::*, Frontier},
	};

	const CONTRACT: H160 = H160::repeat_byte(0x70);

	/// What the runtime's `call` takes.
	type CallArguments = (
		H160,
		H160,
		Vec<u8>,
		U256,
		U256,
		Option<U256>,
		Option<U256>,
		Option<U256>,
		bool,
		Option<Vec<(H160, Vec<H256>)>>,
		Option<Vec<AuthorizationListItem>>,
		StatePayload,
	);

	/// An answer, or the code, message and data of an error.
	type Answer = Result<Value, (i32, String, Option<String>)>;

	/// The gas limit of a recorded runtime `call`.
	fn gas_limit(called: &Called) -> U256 {
		CallArguments::decode(&mut &called.arguments[..]).unwrap().4
	}

	fn ended<T: Encode>(exit_reason: ExitReason, value: T, used_gas: u64) -> Vec<u8> {
		let used_gas = (U256::from(used_gas), U256::from(used_gas));
		let execution = (exit_reason, value, used_gas, None::<[Option<u64>; 4]>, Vec::<ethereum::Log>::new());
		Ok::<_, DispatchError>(execution).encode()
	}

	fn returned(data: &[u8]) -> Vec<u8> {
		ended(ExitReason::Succeed(ExitSucceed::Returned), data.to_vec(), 30_000)
	}

	/// The data of a revert with an `Error(string)` that declares `length`
	/// bytes of reason and carries `reason`.
	fn error_string(length: usize, reason: &[u8]) -> Vec<u8> {
		let mut data = vec![0x08, 0xc3, 0x79, 0xa0];
		data.extend(U256::from(32).to_big_endian());
		data.extend(U256::from(length).to_big_endian());
		data.extend(reason);
		data
	}

	/// A chain whose block 1 runs every `call` to the same end.
	fn chain(call: Vec<u8>) -> FakeNode {
		let mut node = FakeNode::new();
		node.hold(1, U256::one(), Vec::new());
		node.answer(1, MULTIPLIER_SUPPORT, Vec::new(), Vec::new());
		node.answer_with(1, CALL, move |_| call.clone());
		node
	}

	fn ask(node: FakeNode, method: &str, params: Value) -> (Answer, Arc<FakeNode>) {
		let node = Arc::new(node);
		let history = Frontier::new(node.clone()).unwrap();
		let answer = history.call(method, params.as_array().unwrap()).map_err(|error| {
			(error.code(), error.message().to_owned(), error.data().map(|data| data.get().to_owned()))
		});
		(answer, node)
	}

	fn call(node: FakeNode, request: Value) -> Answer {
		ask(node, "eth_call", json!([request, "0x1"])).0
	}

	fn failed(message: &str, data: Option<&str>) -> Answer {
		Err((-32603, message.into(), data.map(|data| format!("\"{data}\""))))
	}

	#[test]
	fn a_call_answers_with_what_the_contract_returned() {
		let answer = call(chain(returned(&[0x12, 0x34])), json!({ "to": CONTRACT }));

		assert_eq!(answer, Ok(json!("0x1234")));
	}

	#[test]
	fn a_revert_reason_is_read_out_of_the_error_string() {
		let plain = "VM Exception while processing transaction: revert";

		assert_eq!(revert_message(&error_string(3, b"abc")), format!("{plain} abc"));
		assert_eq!(revert_message(&error_string(0, b"")), plain);
		assert_eq!(revert_message(&error_string(4, b"abc")), plain);
		assert_eq!(revert_message(&error_string(2, &[0xff, 0xfe])), plain);
		assert_eq!(revert_message(&[]), plain);
	}

	#[test]
	fn a_revert_is_reported_with_its_reason_and_data() {
		let data = error_string(3, b"abc");
		let reverted = ended(ExitReason::Revert(ExitRevert::Reverted), data.clone(), 30_000);

		let answer = call(chain(reverted), json!({ "to": CONTRACT }));

		let data = sp_core::bytes::to_hex(&data, false);
		assert_eq!(answer, failed("VM Exception while processing transaction: revert abc", Some(&data)));
	}

	#[test]
	fn evm_errors_are_reported_by_name() {
		let ending = |reason| call(chain(ended(reason, Vec::<u8>::new(), 30_000)), json!({ "to": CONTRACT }));

		assert_eq!(ending(ExitReason::Error(ExitError::OutOfGas)), failed("out of gas", None));
		assert_eq!(ending(ExitReason::Error(ExitError::OutOfFund)), failed("evm error: OutOfFund", Some("0x")));
		assert_eq!(ending(ExitReason::Fatal(ExitFatal::NotSupported)), failed("evm fatal: NotSupported", Some("0x")));
	}

	#[test]
	fn a_call_may_ask_for_ten_block_gas_limits_and_no_more() {
		let limit = BLOCK_GAS_LIMIT * GAS_LIMIT_MULTIPLIER;
		let asking = |gas: u64| call(chain(returned(&[])), json!({ "to": CONTRACT, "gas": U256::from(gas) }));

		assert_eq!(asking(limit), Ok(json!("0x")));
		assert_eq!(
			asking(limit + 1),
			failed("provided gas limit is too high (can be up to 10x the block gas limit)", None),
		);
	}

	#[test]
	fn a_call_without_a_gas_limit_runs_with_ten_block_gas_limits() {
		let (_, node) = ask(chain(returned(&[])), "eth_call", json!([{ "to": CONTRACT }, "0x1"]));

		let called = node.called.lock().unwrap();
		let run = called.iter().find(|called| called.function == CALL).unwrap();
		assert_eq!(gas_limit(run), U256::from(BLOCK_GAS_LIMIT * GAS_LIMIT_MULTIPLIER));
	}

	#[test]
	fn a_call_runs_with_the_storage_proof_size_metered() {
		let (_, node) = ask(chain(returned(&[])), "eth_call", json!([{ "to": CONTRACT }, "0x1"]));

		let called = node.called.lock().unwrap();
		assert!(called.iter().find(|called| called.function == CALL).unwrap().metered);
	}

	#[test]
	fn gas_price_and_dynamic_fees_together_are_refused() {
		let request = json!({ "to": CONTRACT, "gasPrice": "0x1", "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1" });

		let answer = call(chain(returned(&[])), request);

		assert_eq!(answer, failed("both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified", None));
	}

	#[test]
	fn a_tip_may_match_the_fee_cap_and_not_pass_it() {
		let tipping = |tip: &str| {
			call(chain(returned(&[])), json!({ "to": CONTRACT, "maxFeePerGas": "0x5", "maxPriorityFeePerGas": tip }))
		};

		assert_eq!(tipping("0x5"), Ok(json!("0x")));
		assert_eq!(
			tipping("0x6"),
			failed("Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`", None),
		);
	}

	#[test]
	fn a_transfer_to_an_account_without_code_is_estimated_without_running_it() {
		let mut node = chain(returned(&[]));
		node.answer(1, ACCOUNT_CODE_AT, CONTRACT.encode(), Vec::<u8>::new().encode());

		let (answer, node) = ask(node, "eth_estimateGas", json!([{ "to": CONTRACT, "value": "0x1" }, "0x1"]));

		assert_eq!(answer, Ok(json!("0x5208")));
		assert!(node.called.lock().unwrap().iter().all(|called| called.function != CALL));
	}

	#[test]
	fn a_transfer_to_a_contract_is_estimated_by_running_it() {
		let mut node = chain(returned(&[]));
		node.answer(1, ACCOUNT_CODE_AT, CONTRACT.encode(), vec![0x60u8, 0x00].encode());

		let (answer, _) = ask(node, "eth_estimateGas", json!([{ "to": CONTRACT, "value": "0x1" }, "0x1"]));

		assert_eq!(answer, Ok(json!("0x7530")));
	}

	/// The sender holds 1,000,100 and sends 100 at a gas price of 10, which
	/// leaves enough for 100,000 gas.
	#[test]
	fn an_estimate_runs_with_the_gas_the_sender_can_pay_for() {
		let mut node = chain(returned(&[]));
		node.answer(1, ACCOUNT_BASIC, SENDER.encode(), (U256::from(1_000_100), U256::zero()).encode());
		let request = json!({ "from": SENDER, "to": CONTRACT, "data": "0x00", "value": "0x64", "gasPrice": "0xa" });

		let (_, node) = ask(node, "eth_estimateGas", json!([request, "0x1"]));

		let called = node.called.lock().unwrap();
		assert_eq!(gas_limit(called.iter().find(|called| called.function == CALL).unwrap()), U256::from(100_000));
	}

	#[test]
	fn an_estimate_may_send_the_whole_balance_and_no_more() {
		let sending = |value: u64| {
			let mut node = chain(returned(&[]));
			node.answer(1, ACCOUNT_BASIC, SENDER.encode(), (U256::from(1_000), U256::zero()).encode());
			let request =
				json!({ "from": SENDER, "to": CONTRACT, "data": "0x00", "value": U256::from(value), "gasPrice": "0xa" });
			ask(node, "eth_estimateGas", json!([request, "0x1"])).0
		};

		assert_eq!(sending(1_000), Ok(json!("0x7530")));
		assert_eq!(sending(1_001), failed("insufficient funds for transfer", None));
	}

	#[test]
	fn an_estimate_that_runs_out_of_gas_reports_the_allowance() {
		let out_of_gas = ended(ExitReason::Error(ExitError::OutOfGas), Vec::<u8>::new(), 30_000);
		let request = json!({ "to": CONTRACT, "data": "0x00", "gas": "0x7530" });

		let (answer, _) = ask(chain(out_of_gas), "eth_estimateGas", json!([request, "0x1"]));

		assert_eq!(answer, failed("gas required exceeds allowance 30000", None));
	}

	/// The sender can pay for 100 gas. The call reverts on that and goes
	/// through on the ten block gas limits a call may use.
	#[test]
	fn a_revert_that_more_gas_would_avoid_reports_the_allowance() {
		let mut node = chain(Vec::new());
		node.answer(1, ACCOUNT_BASIC, SENDER.encode(), (U256::from(1_000), U256::zero()).encode());
		node.answer_with(1, CALL, |mut arguments| match CallArguments::decode(&mut arguments).unwrap().4.low_u64() {
			100 => ended(ExitReason::Revert(ExitRevert::Reverted), Vec::<u8>::new(), 100),
			_ => returned(&[]),
		});
		let request = json!({ "from": SENDER, "to": CONTRACT, "data": "0x00", "gasPrice": "0xa" });

		let (answer, _) = ask(node, "eth_estimateGas", json!([request, "0x1"]));

		assert_eq!(answer, failed("gas required exceeds allowance 100", None));
	}

	#[test]
	fn a_revert_that_more_gas_does_not_avoid_is_reported_as_the_revert() {
		let reverted = ended(ExitReason::Revert(ExitRevert::Reverted), error_string(3, b"abc"), 100);
		let mut node = chain(reverted);
		node.answer(1, ACCOUNT_BASIC, SENDER.encode(), (U256::from(1_000), U256::zero()).encode());
		let request = json!({ "from": SENDER, "to": CONTRACT, "data": "0x00", "gasPrice": "0xa" });

		let (answer, _) = ask(node, "eth_estimateGas", json!([request, "0x1"]));

		let Err((_, message, _)) = answer else { panic!("a revert, got {answer:?}") };
		assert_eq!(message, "VM Exception while processing transaction: revert abc");
	}

	/// The keys are where the runtime keeps the account pallet-evm maps the
	/// address to, its code and its storage slot 1.
	#[test]
	fn balance_nonce_code_and_slot_overrides_become_storage_changes() {
		let key = |hex: &str| sp_core::bytes::from_hex(hex).unwrap();
		let account = key("0x26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da97762cfd83147cfc1765a67af7d5cf594decba2ae1f691f902ba17a9dcfb7b91eeb11abc06b0e0748d7cb6be82e0cc195");
		let code = key("0x1da53b775b270400e7e61ed5cbc5a146ea70f53d5a3306ce02aaf97049cf181a9c797704d830bfaa2ac27f94c00d96f77070707070707070707070707070707070707070");
		let slot = key("0x1da53b775b270400e7e61ed5cbc5a146ab1160471b1418779239ba8e2b847e429c797704d830bfaa2ac27f94c00d96f770707070707070707070707070707070707070707af7fa355ea131795dc480485b6c64c90000000000000000000000000000000000000000000000000000000000000001");
		let mut node = chain(returned(&[]));
		node.set_storage(1, account.clone(), vec![0; 80]);
		let overrides = json!({ format!("{CONTRACT:?}"): {
			"balance": "0x5",
			"nonce": "0x9",
			"code": "0x6000",
			"stateDiff": { format!("{:?}", H256::from_low_u64_be(1)): H256::repeat_byte(0x07) },
		} });

		let (_, node) = ask(node, "eth_call", json!([{ "to": CONTRACT }, "0x1", overrides]));

		let mut called = node.called.lock().unwrap();
		let changes = &mut called.iter_mut().find(|called| called.function == CALL).unwrap().changes;
		let info = changes.storage(&account).flatten().unwrap().to_vec();
		assert_eq!((&info[0..4], &info[16..32]), (&9u32.encode()[..], &5u128.encode()[..]));
		assert_eq!(changes.storage(&code).flatten(), Some(&vec![0x60u8, 0x00].encode()[..]));
		assert_eq!(changes.storage(&slot).flatten(), Some(&[0x07; 32][..]));
	}

	#[test]
	fn a_whole_storage_override_goes_to_the_runtime_with_the_call() {
		let slots = json!({ format!("{:?}", H256::from_low_u64_be(1)): H256::repeat_byte(0x07) });
		let overrides = json!({ format!("{CONTRACT:?}"): { "state": slots } });

		let (_, node) = ask(chain(returned(&[])), "eth_call", json!([{ "to": CONTRACT }, "0x1", overrides]));

		let called = node.called.lock().unwrap();
		let run = called.iter().find(|called| called.function == CALL).unwrap();
		let state = CallArguments::decode(&mut &run.arguments[..]).unwrap().11;
		assert_eq!(state, Some(vec![(CONTRACT, vec![(H256::from_low_u64_be(1), H256::repeat_byte(0x07))])]));
	}

	#[test]
	fn replacing_and_patching_the_storage_of_one_account_is_refused() {
		let overrides = json!({ format!("{CONTRACT:?}"): { "state": {}, "stateDiff": {} } });

		let (answer, _) = ask(chain(returned(&[])), "eth_call", json!([{ "to": CONTRACT }, "0x1", overrides]));

		let Err((_, message, _)) = answer else { panic!("a refusal, got {answer:?}") };
		assert!(message.starts_with("both `state` and `stateDiff` specified for account"), "{message}");
	}

	/// The creation writes to storage, and the code is read on top of that.
	#[test]
	fn a_contract_creation_answers_with_the_code_it_left_behind() {
		let created = H160::repeat_byte(0xc0);
		let mut node = chain(Vec::new());
		node.answer_with(1, CREATE, move |_| ended(ExitReason::Succeed(ExitSucceed::Returned), created, 30_000));
		node.write_on(CREATE, b"deployed", b"code");
		node.answer(1, ACCOUNT_CODE_AT, created.encode(), vec![0x60u8, 0x00].encode());

		let (answer, node) = ask(node, "eth_call", json!([{ "data": "0x6000" }, "0x1"]));

		assert_eq!(answer, Ok(json!("0x6000")));
		let mut called = node.called.lock().unwrap();
		let read = called.iter_mut().find(|called| called.function == ACCOUNT_CODE_AT).unwrap();
		assert_eq!(read.changes.storage(b"deployed").flatten(), Some(&b"code"[..]));
	}
}
