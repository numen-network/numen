//! # Pallet Difficulty
//!
//! ASERT difficulty adjustment pallet for PoW consensus.
//!
//! Computes mining difficulty each block using the ASERT algorithm.
//! Difficulty is derived from a fixed anchor block using an exponential
//! formula, eliminating cumulative errors from recursive parent-based
//! adjustments.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

pub mod asert;

mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub mod weights;
pub use weights::*;

sp_api::decl_runtime_apis! {
	/// Runtime API for querying ASERT difficulty parameters and computing
	/// real-time difficulty.
	pub trait DifficultyApi {
		/// Returns (anchor_target, anchor_timestamp_secs, anchor_height,
		/// target_block_time, halflife).
		fn anchor_params() -> (sp_core::U256, u64, u64, u64, u64);

		/// Computes the difficulty a block with the given timestamp must satisfy.
		///
		/// Recomputed from the ASERT anchor rather than read off `CurrentDifficulty`,
		/// which only holds what the last block settled on. Pass the current time to get
		/// the difficulty right now.
		fn realtime_difficulty(block_timestamp_secs: u64) -> sp_core::U256;

		/// Computes a block's difficulty from its timestamp inherent.
		///
		/// `None` on a malformed inherent leaves the block unverifiable, so invalid.
		fn difficulty_for_block(timestamp_inherent: alloc::vec::Vec<u8>) -> Option<sp_core::U256>;
	}
}

#[frame_support::pallet]
pub mod pallet {
	use crate::weights::WeightInfo;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_core::U256;

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_timestamp::Config {
		/// Target block time in seconds (e.g. 20).
		#[pallet::constant]
		type TargetBlockTime: Get<u64>;

		/// ASERT halflife in seconds (e.g. 1800 = 30 minutes).
		#[pallet::constant]
		type Halflife: Get<u64>;

		/// Interruption threshold in seconds.
		#[pallet::constant]
		type BreakThresholdSecs: Get<u64>;

		/// Weight information for this pallet's block hook.
		type WeightInfo: WeightInfo;
	}

	/// Current mining difficulty (U256).
	///
	/// Updated each block by the ASERT calculation in `on_finalize`.
	/// Initially set via genesis config.
	#[pallet::storage]
	#[pallet::getter(fn current_difficulty)]
	pub type CurrentDifficulty<T: Config> = StorageValue<_, U256, ValueQuery>;

	/// Anchor block target value (inverse of difficulty).
	///
	/// `target = U256::MAX / difficulty`. Set at genesis and only
	/// updated when `on_finalize` detects an interruption (a gap from
	/// the parent block exceeding [`Config::BreakThresholdSecs`]).
	#[pallet::storage]
	#[pallet::getter(fn anchor_target)]
	pub type AnchorTarget<T: Config> = StorageValue<_, U256, ValueQuery>;

	/// Timestamp of the anchor block (seconds since Unix epoch).
	///
	/// Auto-initialized on the first block with a valid timestamp,
	/// then only re-set on interruption recovery.
	#[pallet::storage]
	#[pallet::getter(fn anchor_timestamp)]
	pub type AnchorTimestamp<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Block height of the anchor block.
	#[pallet::storage]
	#[pallet::getter(fn anchor_height)]
	pub type AnchorHeight<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Timestamp of the most recently finalized block (seconds).
	///
	/// Used to detect inter-block gaps and decide whether the next
	/// block resumes from an interruption.
	#[pallet::storage]
	#[pallet::getter(fn last_block_timestamp)]
	pub type LastBlockTimestamp<T: Config> = StorageValue<_, u64, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		/// Initial mining difficulty. Must be non-zero.
		pub initial_difficulty: U256,
		#[serde(skip)]
		pub _marker: core::marker::PhantomData<T>,
	}

	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self {
				initial_difficulty: U256::one(),
				_marker: Default::default(),
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			assert!(!self.initial_difficulty.is_zero(), "initial_difficulty must be non-zero");
			CurrentDifficulty::<T>::put(self.initial_difficulty);
			AnchorTarget::<T>::put(U256::MAX / self.initial_difficulty);
		}
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The difficulty value overflowed or underflowed during calculation.
		DifficultyOverflow,
		/// Zero difficulty is not allowed.
		ZeroDifficulty,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		// `on_finalize` has no way to report what it spent, so the ASERT
		// update is paid for up front here.
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			<T as Config>::WeightInfo::on_finalize()
		}

		fn on_finalize(_n: BlockNumberFor<T>) {
			let current_height: u64 = frame_system::Pallet::<T>::block_number()
										.try_into()
										.unwrap_or(u64::MAX);

			// Timestamp is in milliseconds; convert to seconds.
			// In on_finalize the timestamp inherent has already executed,
			// so this returns the *current* block's timestamp.
			let now_ms: u64 = pallet_timestamp::Pallet::<T>::get()
								.try_into()
								.unwrap_or(0u64);
			let now_secs = now_ms / 1000;

			// Auto-initialize anchor on the first block with a valid
			// timestamp. This block becomes the anchor — keep the
			// initial difficulty unchanged.
			let anchor_ts = AnchorTimestamp::<T>::get();
			if anchor_ts == 0 && now_secs > 0 {
				AnchorTimestamp::<T>::put(now_secs);
				AnchorHeight::<T>::put(current_height);
				LastBlockTimestamp::<T>::put(now_secs);
				return;
			}

			let anchor_height = AnchorHeight::<T>::get();

			// Skip ASERT on the anchor block itself.
			if current_height <= anchor_height {
				LastBlockTimestamp::<T>::put(now_secs);
				return;
			}

			let height_delta = current_height.saturating_sub(anchor_height);
			let time_delta = (now_secs as i128) - (anchor_ts as i128);

			let anchor_target = AnchorTarget::<T>::get();
			if anchor_target.is_zero() {
				LastBlockTimestamp::<T>::put(now_secs);
				return;
			}

			let next_target = crate::asert::compute_next_target(
				anchor_target,
				time_delta,
				height_delta,
				T::TargetBlockTime::get(),
				T::Halflife::get(),
			);

			let new_difficulty = U256::MAX / next_target;

			CurrentDifficulty::<T>::put(new_difficulty);

			// Interruption recovery: if the gap from the parent block
			// exceeded `BreakThresholdSecs`, re-anchor onto the just-
			// finalized block. This resets `height_delta` so that
			// subsequent blocks compute their target relative to the
			// recovery block rather than having to "pay back" the long
			// outage gap through many fast catch-up blocks.
			let last_ts = LastBlockTimestamp::<T>::get();
			let block_gap = now_secs.saturating_sub(last_ts);
			if block_gap > T::BreakThresholdSecs::get() {
				AnchorTarget::<T>::put(next_target);
				AnchorTimestamp::<T>::put(now_secs);
				AnchorHeight::<T>::put(current_height);
			}

			LastBlockTimestamp::<T>::put(now_secs);
		}
	}
}

// ── Public helpers ──────────────────────────────────────────────────

use frame_support::pallet_prelude::Get;
use sp_core::U256;

impl<T: pallet::Config> Pallet<T> {
	/// Computes the difficulty a block with the given timestamp must satisfy.
	///
	/// Recomputed from the ASERT anchor rather than read off `CurrentDifficulty`,
	/// which only holds what the last block settled on. Pass the current time to get
	/// the difficulty right now.
	pub fn realtime_difficulty(block_timestamp_secs: u64) -> U256 {
		let current_height: u64 = frame_system::Pallet::<T>::block_number()
			.try_into()
			.unwrap_or(u64::MAX);
		let next_height = current_height.saturating_add(1);
		let anchor_height = AnchorHeight::<T>::get();
		let anchor_ts = AnchorTimestamp::<T>::get();
		let anchor_target = AnchorTarget::<T>::get();

		let height_delta = next_height.saturating_sub(anchor_height);
		let time_delta = (block_timestamp_secs as i128) - (anchor_ts as i128);

		let next_target = crate::asert::compute_next_target(
			anchor_target,
			time_delta,
			height_delta,
			T::TargetBlockTime::get(),
			T::Halflife::get(),
		);

		U256::MAX / next_target
	}
}
