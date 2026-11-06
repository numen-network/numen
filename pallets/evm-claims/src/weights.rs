#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]
#![allow(dead_code)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for `pallet_evm_claims`.
pub trait WeightInfo {
	fn claim(s: u32) -> Weight;
	fn authorize_claim() -> Weight;
}

/// Upper bounds from the storage the calls touch. `s` is the number of
/// vesting schedules a claim moves, each on its own.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	fn claim(s: u32) -> Weight {
		Weight::from_parts(60_000_000, 12_000)
			.saturating_add(Weight::from_parts(5_000_000, 1_000).saturating_mul(s.into()))
			.saturating_add(T::DbWeight::get().reads(14_u64))
			.saturating_add(T::DbWeight::get().reads(2_u64).saturating_mul(s.into()))
			.saturating_add(T::DbWeight::get().writes(10_u64))
			.saturating_add(T::DbWeight::get().writes(4_u64).saturating_mul(s.into()))
	}
	fn authorize_claim() -> Weight {
		Weight::from_parts(20_000_000, 10_000)
			.saturating_add(T::DbWeight::get().reads(8_u64))
	}
}

impl WeightInfo for () {
	fn claim(s: u32) -> Weight {
		Weight::from_parts(60_000_000, 12_000)
			.saturating_add(Weight::from_parts(5_000_000, 1_000).saturating_mul(s.into()))
			.saturating_add(RocksDbWeight::get().reads(14_u64))
			.saturating_add(RocksDbWeight::get().reads(2_u64).saturating_mul(s.into()))
			.saturating_add(RocksDbWeight::get().writes(10_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64).saturating_mul(s.into()))
	}
	fn authorize_claim() -> Weight {
		Weight::from_parts(20_000_000, 10_000)
			.saturating_add(RocksDbWeight::get().reads(8_u64))
	}
}
