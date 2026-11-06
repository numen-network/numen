//! Benchmarks for the EVM claims pallet.

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use frame_benchmarking::v2::*;
use frame_support::traits::{
	fungible::{Inspect, Mutate},
	Authorize, VestingSchedule,
};
use frame_system::RawOrigin;
use sp_core::H160;
use sp_runtime::{traits::Bounded, transaction_validity::TransactionSource};

/// An old account holding funds and a nonce under `schedules` vesting
/// schedules, each of which a claim moves on its own.
fn legacy_with<T: Config>(schedules: u32) -> H160 {
	let address = H160::repeat_byte(0x42);
	let legacy = Pallet::<T>::legacy_account(&address);
	let unit = pallet_balances::Pallet::<T>::minimum_balance().max(1u32.into());
	let amount = T::Balance::max_value() / 4u32.into();
	pallet_balances::Pallet::<T>::set_balance(&legacy, amount);
	for _ in 0..schedules {
		pallet_vesting::Pallet::<T>::add_vesting_schedule(
			&legacy,
			unit * 1_000u32.into(),
			unit,
			frame_system::Pallet::<T>::block_number(),
		)
		.expect("the account has room for another schedule");
	}
	frame_system::Account::<T>::mutate(&legacy, |info| info.nonce = 7u32.into());
	address
}

#[benchmarks(where <T as frame_system::Config>::RuntimeCall: From<Call<T>>)]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn claim(s: Linear<0, { T::MAX_VESTING_SCHEDULES }>) {
		let address = legacy_with::<T>(s);

		#[extrinsic_call]
		_(RawOrigin::Authorized, address);

		assert!(!Pallet::<T>::claimable(&address));
	}

	#[benchmark]
	fn authorize_claim() -> Result<(), BenchmarkError> {
		let address = legacy_with::<T>(T::MAX_VESTING_SCHEDULES);
		let call: <T as frame_system::Config>::RuntimeCall = Call::claim { address }.into();

		#[block]
		{
			call.authorize(TransactionSource::External)
				.expect("claim carries authorize logic")
				.map_err(|_| "authorize failed")?;
		}

		Ok(())
	}

	impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}
