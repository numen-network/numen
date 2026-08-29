// Tests for the Frontier EVM stack wired into this runtime.
//
// These pin the `AccountId32 ↔ H160` address mapping and exercise contract
// deployment plus the `balances-erc20` precompile through real
// `pallet_evm::runner::stack::Runner` calls.

mod common;

use common::new_test_ext;
use fp_evm::MAX_TRANSACTION_GAS_LIMIT;
use frame_support::traits::tokens::fungible::Mutate;
use pallet_evm::{AddressMapping, Runner};
use numen_runtime::{
	configs::TreasuryAccount, AccountId, Balances, Runtime, EXISTENTIAL_DEPOSIT, UNIT,
};
use sp_core::{H160, U256};

/// Twice the `DefaultBaseFeePerGas`.
const MAX_FEE_PER_GAS: u64 = 2_000_000_000_000;
const GAS_LIMIT: u64 = 1_000_000;

/// Smallest balance an EVM caller can hold and still transact. `pallet-evm`
/// reads spendable balance under `Preservation::Preserve`, so the caller funds
/// the whole gas prepay on top of an untouchable existential deposit.
const MIN_CALLER_BALANCE: u128 =
	GAS_LIMIT as u128 * MAX_FEE_PER_GAS as u128 + EXISTENTIAL_DEPOSIT;

/// The chain-specific `balances-erc20` precompile.
const ERC20_PRECOMPILE: u64 = 0x0802;

/// Init bytecode that returns a one byte runtime of `STOP` (0x00).
///
///   60 01  PUSH1 0x01   size of runtime code
///   60 0c  PUSH1 0x0c   offset in code
///   60 00  PUSH1 0x00   memory destination
///   39     CODECOPY
///   60 01  PUSH1 0x01   return size
///   60 00  PUSH1 0x00   return offset
///   f3     RETURN
///   00     STOP         runtime code byte 12
const MINIMAL_INIT_CODE: [u8; 13] =
	[0x60, 0x01, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x01, 0x60, 0x00, 0xf3, 0x00];

fn evm_account(addr: H160) -> AccountId {
	<Runtime as pallet_evm::Config>::AddressMapping::into_account_id(addr)
}

fn create_minimal_contract(
	caller: H160,
	gas_limit: u64,
) -> Result<fp_evm::CreateInfo, pallet_evm::Error<Runtime>> {
	<Runtime as pallet_evm::Config>::Runner::create(
		caller,
		MINIMAL_INIT_CODE.to_vec(),
		U256::zero(),
		gas_limit,
		Some(U256::from(MAX_FEE_PER_GAS)),
		None,
		None,
		Vec::new(),
		Vec::new(),
		true,
		true,
		None,
		None,
		<Runtime as pallet_evm::Config>::config(),
	)
	.map_err(|e| e.error)
}

/// Drive a `Runner::call` into the balances-erc20 precompile, returning its raw
/// ABI encoded output. Panics unless the call succeeds.
fn call_erc20_precompile(caller: H160, data: Vec<u8>) -> Vec<u8> {
	let res = <Runtime as pallet_evm::Config>::Runner::call(
		caller,
		H160::from_low_u64_be(ERC20_PRECOMPILE),
		data,
		U256::zero(),
		GAS_LIMIT,
		Some(U256::from(MAX_FEE_PER_GAS)),
		None,
		None,
		Vec::new(),
		Vec::new(),
		true,
		true,
		None,
		None,
		None,
		<Runtime as pallet_evm::Config>::config(),
	)
	.expect("precompile call must dispatch without runtime error");
	assert!(res.exit_reason.is_succeed(), "precompile call must succeed: {:?}", res.exit_reason);
	res.value
}

/// `balanceOf(address)`, selector `keccak256(...)[..4]`.
fn encode_balance_of(who: H160) -> Vec<u8> {
	let mut data = vec![0x70, 0xa0, 0x82, 0x31];
	data.extend_from_slice(&[0u8; 12]);
	data.extend_from_slice(who.as_bytes());
	data
}

/// `transfer(address,uint256)`.
fn encode_transfer(to: H160, amount: u128) -> Vec<u8> {
	let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
	data.extend_from_slice(&[0u8; 12]);
	data.extend_from_slice(to.as_bytes());
	data.extend_from_slice(&U256::from(amount).to_big_endian());
	data
}

/// `withdraw(bytes32,uint256)`.
fn encode_withdraw(dest: [u8; 32], amount: u128) -> Vec<u8> {
	let mut data = vec![0x04, 0x0c, 0xf0, 0x20];
	data.extend_from_slice(&dest);
	data.extend_from_slice(&U256::from(amount).to_big_endian());
	data
}

#[test]
fn address_mapping_matches_frontier_golden_vector() {
	// Frontier's `HashedAddressMapping<BlakeTwo256>` derives the substrate
	// account as `blake2_256("evm:" ++ h160)`. A golden vector pins the choice
	// without restating the derivation, so swapping the mapping turns this red.
	let mapped = evm_account(H160::repeat_byte(0xAB));
	let expected: AccountId = sp_runtime::AccountId32::from([
		0x82, 0xf8, 0xf7, 0x89, 0x05, 0xbc, 0x46, 0xfd, 0xa2, 0xe4, 0xe1, 0xdd, 0x8d, 0x61, 0x19,
		0xe3, 0x2a, 0xb4, 0x40, 0x8c, 0x3b, 0xf7, 0x8a, 0xe8, 0xab, 0x3f, 0xf2, 0x96, 0xb3, 0x8d,
		0x28, 0x6e,
	]);

	assert_eq!(mapped, expected);
}

#[test]
fn deploy_minimal_contract_succeeds() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xCAFE);
		Balances::set_balance(&evm_account(caller), 1_000 * UNIT);

		let info = create_minimal_contract(caller, GAS_LIMIT)
			.expect("EVM create must dispatch without runtime error");

		assert!(
			info.exit_reason.is_succeed(),
			"contract creation must succeed: {:?}",
			info.exit_reason
		);
		assert_eq!(
			pallet_evm::AccountCodes::<Runtime>::get(info.value),
			vec![0x00],
			"deployed runtime code must be STOP"
		);
	});
}

#[test]
fn create_below_minimum_caller_balance_is_rejected() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xCAFE);
		// One wei short of covering the gas prepay without dipping into the ED.
		Balances::set_balance(&evm_account(caller), MIN_CALLER_BALANCE - 1);

		let err = create_minimal_contract(caller, GAS_LIMIT)
			.expect_err("caller that cannot cover the gas prepay must be rejected");

		assert!(matches!(err, pallet_evm::Error::BalanceLow), "unexpected error: {err:?}");
	});
}

#[test]
fn create_at_minimum_caller_balance_succeeds() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xCAFE);
		Balances::set_balance(&evm_account(caller), MIN_CALLER_BALANCE);

		let info = create_minimal_contract(caller, GAS_LIMIT)
			.expect("caller holding exactly the minimum is accepted");

		assert!(info.exit_reason.is_succeed(), "unexpected exit: {:?}", info.exit_reason);
	});
}

#[test]
fn create_at_transaction_gas_cap_succeeds() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xCAFE);
		Balances::set_balance(&evm_account(caller), 1_000 * UNIT);

		let info = create_minimal_contract(caller, MAX_TRANSACTION_GAS_LIMIT.as_u64())
			.expect("gas limit sitting on the cap is accepted");

		assert!(info.exit_reason.is_succeed(), "unexpected exit: {:?}", info.exit_reason);
	});
}

#[test]
fn create_above_transaction_gas_cap_is_rejected() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xCAFE);
		Balances::set_balance(&evm_account(caller), 1_000 * UNIT);

		let err = create_minimal_contract(caller, MAX_TRANSACTION_GAS_LIMIT.as_u64() + 1)
			.expect_err("gas limit one above the cap must be rejected");

		assert!(
			matches!(err, pallet_evm::Error::TransactionGasLimitExceedsCap),
			"unexpected error: {err:?}"
		);
	});
}

#[test]
fn balances_erc20_precompile_reports_native_balance() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xC1A1);
		let holder = H160::from_low_u64_be(0xB0B0);
		let holder_balance = 4_242 * UNIT;
		Balances::set_balance(&evm_account(caller), 100 * UNIT);
		Balances::set_balance(&evm_account(holder), holder_balance);

		// Query a third party rather than the caller. The gas prepay is already
		// docked from the caller's mirror account by the time the precompile
		// body runs, so only a non-paying account exposes the balance exactly.
		let ret = call_erc20_precompile(caller, encode_balance_of(holder));

		assert_eq!(ret.len(), 32, "balanceOf returns one ABI word");
		assert_eq!(U256::from_big_endian(&ret), U256::from(holder_balance));
	});
}

#[test]
fn balances_erc20_precompile_transfer_moves_native_funds() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xC1A1);
		let recipient = H160::from_low_u64_be(0xB0B0);
		let caller_acc = evm_account(caller);
		let recipient_acc = evm_account(recipient);

		let initial = 7_500 * UNIT;
		let amount = 1_000 * UNIT;
		Balances::set_balance(&caller_acc, initial);
		let treasury_before = Balances::free_balance(TreasuryAccount::get());

		let ret = call_erc20_precompile(caller, encode_transfer(recipient, amount));
		assert_eq!(U256::from_big_endian(&ret), U256::one(), "transfer returns bool true");

		// Everything leaving the caller is either transferred or paid as gas,
		// and gas from this authorless block lands in the treasury.
		let gas_paid = Balances::free_balance(TreasuryAccount::get()) - treasury_before;
		assert_eq!(Balances::free_balance(&recipient_acc), amount);
		assert_eq!(
			initial - Balances::free_balance(&caller_acc),
			amount + gas_paid,
			"caller debit must equal amount plus gas"
		);
	});
}

#[test]
fn balances_erc20_precompile_withdraw_moves_funds_back_to_substrate() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xC1A1);
		let caller_acc = evm_account(caller);

		// A pure substrate destination, i.e. an `AccountId32` with no
		// `HashedAddressMapping(H160)` preimage. Mirrors withdrawing to a
		// keyring account or sudo.
		let dest_bytes = [0x42u8; 32];
		let dest_acc: AccountId = sp_runtime::AccountId32::from(dest_bytes);

		let initial = 7_500 * UNIT;
		let amount = 1_000 * UNIT;
		Balances::set_balance(&caller_acc, initial);
		assert_eq!(Balances::free_balance(&dest_acc), 0);
		let treasury_before = Balances::free_balance(TreasuryAccount::get());

		let ret = call_erc20_precompile(caller, encode_withdraw(dest_bytes, amount));
		assert_eq!(U256::from_big_endian(&ret), U256::one(), "withdraw returns bool true");

		let gas_paid = Balances::free_balance(TreasuryAccount::get()) - treasury_before;
		assert_eq!(
			Balances::free_balance(&dest_acc),
			amount,
			"destination substrate account must be credited by amount"
		);
		assert_eq!(
			initial - Balances::free_balance(&caller_acc),
			amount + gas_paid,
			"caller debit must equal amount plus gas"
		);
	});
}
