//! Storage migrations run by the next runtime upgrade.

use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};
use pallet_transaction_payment::{Multiplier, NextFeeMultiplier};
use sp_runtime::{FixedPointNumber, Saturating};

use crate::{
	configs::{MinimumMultiplier, FEE_ANCHOR},
	Runtime,
};

/// Moves the stored fee multiplier onto the [`FEE_ANCHOR`] scale, which keeps
/// the weight and byte prices users pay where they were.
///
/// A multiplier stored against an anchor of one sits below the new floor, so
/// only such a value gets scaled and a second run leaves it alone.
pub struct AnchorFeeMultiplier;

impl AnchorFeeMultiplier {
	fn anchored(stored: Multiplier) -> Multiplier {
		if stored < MinimumMultiplier::get() {
			stored.saturating_mul(Multiplier::saturating_from_integer(FEE_ANCHOR))
		} else {
			stored
		}
	}
}

impl OnRuntimeUpgrade for AnchorFeeMultiplier {
	fn on_runtime_upgrade() -> Weight {
		let db = <Runtime as frame_system::Config>::DbWeight::get();
		let stored = NextFeeMultiplier::<Runtime>::get();
		let anchored = Self::anchored(stored);
		if anchored == stored {
			return db.reads(1);
		}
		NextFeeMultiplier::<Runtime>::put(anchored);
		db.reads_writes(1, 1)
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade() -> Result<alloc::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
		Ok(codec::Encode::encode(&NextFeeMultiplier::<Runtime>::get()))
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(state: alloc::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
		let before = <Multiplier as codec::Decode>::decode(&mut &state[..])
			.map_err(|_| "pre_upgrade state does not decode")?;
		frame_support::ensure!(
			NextFeeMultiplier::<Runtime>::get() == Self::anchored(before),
			"fee multiplier left off the anchor scale"
		);
		Ok(())
	}
}
