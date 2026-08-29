//! Fee routing and calibration. Substrate fees and the EVM base fee split
//! between the PoW author from the block digest and the treasury, and tips
//! reach the author whole. A block without an author pays everything to the
//! treasury. The substrate fee constants stay pinned to the EVM gas price so
//! neither path undercuts the other.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{
	dispatch::DispatchClass,
	weights::Weight,
	traits::{
		tokens::{
			fungible::{Balanced, Mutate},
			Fortitude, Precision, Preservation,
		},
		OnUnbalanced,
	},
};
use numen_runtime::{
	configs::{
		evm::DefaultBaseFeePerGas, DealWithFees, FullnessFeeUpdate, RuntimeBlockLength,
		RuntimeBlockWeights, TreasuryAccount, LENGTH_FEE, MINER_FEE_SHARE, WEIGHT_FEE,
	},
	AccountId, Balance, Balances, Runtime, System, TransactionPayment, UNIT,
};
use pallet_evm::{AddressMapping, FeeCalculator, Runner};
use pallet_transaction_payment::Multiplier;
use sp_consensus_pow::POW_ENGINE_ID;
use sp_core::{H160, U256};
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	traits::{Convert, One},
	DigestItem, FixedPointNumber,
};

/// Twice the base fee, leaving room for the tip below.
const MAX_FEE_PER_GAS: u64 = 2_000_000_000_000;
const TIP_PER_GAS: u64 = 500_000_000_000;
const GAS_LIMIT: u64 = 100_000;
const CALLER_FUNDS: Balance = 1_000 * UNIT;

fn miner() -> AccountId {
	Sr25519Keyring::Eve.to_account_id()
}

/// Stamp the block digest with a PoW author, the way both miners do.
fn set_pow_author(author: &AccountId) {
	System::deposit_log(DigestItem::PreRuntime(POW_ENGINE_ID, author.encode()));
}

fn evm_account(addr: H160) -> AccountId {
	<Runtime as pallet_evm::Config>::AddressMapping::into_account_id(addr)
}

/// Withdraw `fee` from the payer exactly like `FungibleAdapter` does before it
/// hands the credit to the fee sink.
fn fee_credit(
	payer: &AccountId,
	fee: Balance,
) -> frame_support::traits::fungible::Credit<AccountId, Balances> {
	<Balances as Balanced<AccountId>>::withdraw(
		payer,
		fee,
		Precision::Exact,
		Preservation::Expendable,
		Fortitude::Polite,
	)
	.expect("payer covers the fee")
}

/// A transactional EVM call to a plain address, paying a tip on top of the
/// base fee.
fn evm_plain_call_with_tip(caller: H160) -> fp_evm::CallInfo {
	<Runtime as pallet_evm::Config>::Runner::call(
		caller,
		H160::from_low_u64_be(0xD00D),
		Vec::new(),
		U256::zero(),
		GAS_LIMIT,
		Some(U256::from(MAX_FEE_PER_GAS)),
		Some(U256::from(TIP_PER_GAS)),
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
	.expect("plain call must dispatch without runtime error")
}

#[test]
fn substrate_fees_split_between_the_author_and_the_treasury() {
	new_test_ext().execute_with(|| {
		let author = miner();
		set_pow_author(&author);
		let payer = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&payer, CALLER_FUNDS);
		let issuance_before = Balances::total_issuance();
		let fee = UNIT;
		let kept = MINER_FEE_SHARE * fee;

		DealWithFees::on_nonzero_unbalanced(fee_credit(&payer, fee));

		assert_eq!(
			Balances::free_balance(&author),
			kept,
			"the digest author keeps its share and no more",
		);
		assert_eq!(
			Balances::free_balance(TreasuryAccount::get()),
			fee - kept,
			"what the author does not keep funds the treasury",
		);
		assert_eq!(Balances::total_issuance(), issuance_before, "no part of the fee burns");
	});
}

/// A miner filling its own block pays the full fee and gets back only its
/// share, so the round trip has to lose money.
#[test]
fn a_self_paid_fee_costs_the_miner_more_than_it_returns() {
	new_test_ext().execute_with(|| {
		let author = miner();
		set_pow_author(&author);
		Balances::set_balance(&author, CALLER_FUNDS);
		let fee = UNIT;

		DealWithFees::on_nonzero_unbalanced(fee_credit(&author, fee));

		assert!(
			Balances::free_balance(&author) < CALLER_FUNDS,
			"paying itself must not come out even",
		);
	});
}

#[test]
fn substrate_tips_reach_the_author_whole() {
	new_test_ext().execute_with(|| {
		let author = miner();
		set_pow_author(&author);
		let payer = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&payer, CALLER_FUNDS);
		let fee = UNIT;
		let tip = UNIT;

		DealWithFees::on_unbalanceds(
			[fee_credit(&payer, fee), fee_credit(&payer, tip)].into_iter(),
		);

		assert_eq!(
			Balances::free_balance(&author),
			MINER_FEE_SHARE * fee + tip,
			"the tip reaches the author whole while the fee splits",
		);
	});
}

#[test]
fn substrate_fees_reach_the_treasury_without_author_digest() {
	new_test_ext().execute_with(|| {
		let payer = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&payer, CALLER_FUNDS);
		let issuance_before = Balances::total_issuance();
		let fee = UNIT;

		DealWithFees::on_nonzero_unbalanced(fee_credit(&payer, fee));

		assert_eq!(
			Balances::free_balance(TreasuryAccount::get()),
			fee,
			"an authorless fee credit funds the treasury in full",
		);
		assert_eq!(Balances::total_issuance(), issuance_before, "no part of the fee burns");
	});
}

#[test]
fn substrate_tips_reach_the_treasury_without_author_digest() {
	new_test_ext().execute_with(|| {
		let payer = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&payer, CALLER_FUNDS);
		let issuance_before = Balances::total_issuance();
		let fee = UNIT;
		let tip = UNIT;

		DealWithFees::on_unbalanceds(
			[fee_credit(&payer, fee), fee_credit(&payer, tip)].into_iter(),
		);

		assert_eq!(
			Balances::free_balance(TreasuryAccount::get()),
			fee + tip,
			"fee and tip both fund the treasury when the block has no author",
		);
		assert_eq!(Balances::total_issuance(), issuance_before, "no part of the fee burns");
	});
}

#[test]
fn evm_base_fee_splits_while_the_tip_reaches_the_author() {
	new_test_ext().execute_with(|| {
		let author = miner();
		set_pow_author(&author);
		let caller = H160::from_low_u64_be(0xFEE1);
		let caller_acc = evm_account(caller);
		Balances::set_balance(&caller_acc, CALLER_FUNDS);
		let issuance_before = Balances::total_issuance();
		let (base_fee, _) = <Runtime as pallet_evm::Config>::FeeCalculator::min_gas_price();

		let info = evm_plain_call_with_tip(caller);
		assert!(info.exit_reason.is_succeed(), "unexpected exit: {:?}", info.exit_reason);

		let base_paid = (info.used_gas.effective * base_fee).as_u128();
		let tip_paid = (info.used_gas.effective * U256::from(TIP_PER_GAS)).as_u128();
		let caller_spent = CALLER_FUNDS - Balances::free_balance(&caller_acc);
		let author_gain = Balances::free_balance(&author);

		assert_eq!(
			author_gain,
			MINER_FEE_SHARE * base_paid + tip_paid,
			"the author takes its share of the base fee and the whole tip",
		);
		assert_eq!(caller_spent, base_paid + tip_paid, "the caller pays both in full");
		assert_eq!(
			Balances::free_balance(evm_account(H160::zero())),
			0,
			"the tip must bypass the zero coinbase the default handler pays",
		);
		assert_eq!(
			Balances::free_balance(TreasuryAccount::get()),
			base_paid - MINER_FEE_SHARE * base_paid,
			"the base fee the author does not keep funds the treasury",
		);
		assert_eq!(Balances::total_issuance(), issuance_before, "no part of the fee burns");
	});
}

#[test]
fn evm_fees_reach_the_treasury_without_author_digest() {
	new_test_ext().execute_with(|| {
		let caller = H160::from_low_u64_be(0xFEE1);
		let caller_acc = evm_account(caller);
		Balances::set_balance(&caller_acc, CALLER_FUNDS);
		let issuance_before = Balances::total_issuance();

		let info = evm_plain_call_with_tip(caller);
		assert!(info.exit_reason.is_succeed(), "unexpected exit: {:?}", info.exit_reason);

		let caller_spent = CALLER_FUNDS - Balances::free_balance(&caller_acc);
		assert!(caller_spent > 0);
		assert_eq!(
			Balances::free_balance(TreasuryAccount::get()),
			caller_spent,
			"base fee and tip both fund the treasury in an authorless block",
		);
		assert_eq!(Balances::total_issuance(), issuance_before, "no part of the fee burns");
	});
}

/// `WeightPerGas` derives from the block gas limit and the block weight budget,
/// so retuning either one silently reprices substrate compute against EVM
/// compute.
#[test]
fn weight_fee_tracks_the_evm_gas_price() {
	let weight_per_gas =
		Balance::from(<Runtime as pallet_evm::Config>::WeightPerGas::get().ref_time());

	assert_eq!(
		WEIGHT_FEE * weight_per_gas,
		DefaultBaseFeePerGas::get().as_u128(),
		"one gas unit of work must cost the same through either path",
	);
}

/// A block fills on whichever of weight or length runs out first, so the two
/// have to cost about the same. Otherwise the cheaper dimension carries the
/// spam.
#[test]
fn weight_and_length_price_a_full_block_alike() {
	let normal_weight = RuntimeBlockWeights::get()
		.get(DispatchClass::Normal)
		.max_total
		.expect("normal class is bounded")
		.ref_time();
	let normal_length = *RuntimeBlockLength::get().max.get(DispatchClass::Normal);

	let weight_cost = WEIGHT_FEE * Balance::from(normal_weight);
	let length_cost = LENGTH_FEE * Balance::from(normal_length);

	assert!(
		weight_cost <= 2 * length_cost && length_cost <= 2 * weight_cost,
		"full block costs drifted apart, weight {weight_cost} vs length {length_cost}",
	);
}


/// The dimension nearest its ceiling sets the multiplier. A block packed with
/// bytes barely touches the weight budget, so weight alone would read it as an
/// idle chain and cut the price while the block is in fact full.
#[test]
fn bytes_alone_raise_the_multiplier() {
	new_test_ext().execute_with(|| {
		let max_length = *RuntimeBlockLength::get().max.get(DispatchClass::Normal);
		System::set_block_consumed_resources(Weight::zero(), max_length as usize);

		assert!(
			FullnessFeeUpdate::<Runtime>::convert(Multiplier::one()) > Multiplier::one(),
			"a length full block has to raise the next block's price",
		);
	});
}

#[test]
fn weight_alone_raises_the_multiplier() {
	new_test_ext().execute_with(|| {
		let max_weight = RuntimeBlockWeights::get()
			.get(DispatchClass::Normal)
			.max_total
			.expect("normal class is bounded");
		System::set_block_consumed_resources(max_weight, 0);

		assert!(
			FullnessFeeUpdate::<Runtime>::convert(Multiplier::one()) > Multiplier::one(),
			"a weight full block has to raise the next block's price",
		);
	});
}

#[test]
fn an_empty_block_lowers_the_multiplier() {
	new_test_ext().execute_with(|| {
		System::set_block_consumed_resources(Weight::zero(), 0);

		assert!(
			FullnessFeeUpdate::<Runtime>::convert(Multiplier::one()) < Multiplier::one(),
			"an idle chain has to get cheaper",
		);
	});
}

/// Congestion has to reach the byte price too. Left flat, it would be the one
/// dimension an attacker can saturate at a price that never answers back.
#[test]
fn the_byte_price_follows_the_multiplier() {
	new_test_ext().execute_with(|| {
		let bytes = 1_000;
		let flat = TransactionPayment::length_to_fee(bytes);

		pallet_transaction_payment::NextFeeMultiplier::<Runtime>::put(
			Multiplier::saturating_from_integer(4),
		);

		assert_eq!(
			TransactionPayment::length_to_fee(bytes),
			flat * 4,
			"four times the multiplier is four times the byte price",
		);
	});
}
