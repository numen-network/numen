//! The upgrade that anchors the fee multiplier. A multiplier stored against an
//! anchor of one moves onto the new scale so the weight and byte prices users
//! pay stay put, and a value already on the scale is left alone.

mod common;

use common::new_test_ext;
use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};
use numen_runtime::{
	configs::{FEE_ANCHOR, LENGTH_FEE, WEIGHT_FEE},
	migrations::AnchorFeeMultiplier,
	Balance, Runtime, TransactionPayment,
};
use pallet_transaction_payment::{Multiplier, NextFeeMultiplier};
use sp_runtime::FixedPointNumber;

fn migrate(stored: Multiplier) -> Multiplier {
	NextFeeMultiplier::<Runtime>::put(stored);
	AnchorFeeMultiplier::on_runtime_upgrade();
	NextFeeMultiplier::<Runtime>::get()
}

/// The old floor, anchor and ceiling land on the new floor, anchor and
/// ceiling.
#[test]
fn the_old_range_lands_on_the_anchored_range() {
	new_test_ext().execute_with(|| {
		for (old, anchored) in [
			(Multiplier::saturating_from_rational(1, 10), FEE_ANCHOR / 10),
			(Multiplier::saturating_from_integer(1), FEE_ANCHOR),
			(Multiplier::saturating_from_integer(10), FEE_ANCHOR * 10),
		] {
			assert_eq!(migrate(old), Multiplier::saturating_from_integer(anchored), "old {old:?}");
		}
	});
}

/// Whatever congestion left the multiplier at, users pay the same weight and
/// byte prices on both sides of the upgrade.
#[test]
fn the_migration_keeps_the_weight_and_byte_prices() {
	new_test_ext().execute_with(|| {
		let old = Multiplier::saturating_from_rational(5_437, 1_000);
		let ref_time = 1_000_000;
		let bytes = 1_000;

		migrate(old);

		let unadjusted = TransactionPayment::weight_to_fee(Weight::from_parts(ref_time, 0));
		assert_eq!(
			NextFeeMultiplier::<Runtime>::get().saturating_mul_int(unadjusted),
			old.saturating_mul_int(WEIGHT_FEE * Balance::from(ref_time)),
		);
		assert_eq!(
			TransactionPayment::length_to_fee(bytes),
			old.saturating_mul_int(LENGTH_FEE * Balance::from(bytes)),
		);
	});
}

/// A value already on the anchored scale, down to the floor itself, is what a
/// second run of the upgrade sees, and it stays where it is.
#[test]
fn an_anchored_multiplier_is_left_alone() {
	new_test_ext().execute_with(|| {
		for anchored in [FEE_ANCHOR / 10, FEE_ANCHOR] {
			let stored = Multiplier::saturating_from_integer(anchored);
			assert_eq!(migrate(stored), stored);
		}
	});
}
