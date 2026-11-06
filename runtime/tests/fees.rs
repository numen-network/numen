//! Fee routing and calibration. Transaction fees split between the PoW author
//! from the block digest and the treasury, and tips reach the author whole. A
//! block without an author pays everything to the treasury. Weight and length
//! price a full block alike, and filling either one raises the next block's
//! price.

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
		DealWithFees, FullnessFeeUpdate, RuntimeBlockLength, RuntimeBlockWeights,
		TreasuryAccount, LENGTH_FEE, MINER_FEE_SHARE, WEIGHT_FEE,
	},
	AccountId, Balance, Balances, Runtime, System, TransactionPayment, UNIT,
};
use pallet_transaction_payment::Multiplier;
use sp_consensus_pow::POW_ENGINE_ID;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	traits::{Convert, One},
	DigestItem, FixedPointNumber,
};

const CALLER_FUNDS: Balance = 1_000 * UNIT;

fn miner() -> AccountId {
	Sr25519Keyring::Eve.to_account_id()
}

/// Stamp the block digest with a PoW author, the way both miners do.
fn set_pow_author(author: &AccountId) {
	System::deposit_log(DigestItem::PreRuntime(POW_ENGINE_ID, author.encode()));
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
