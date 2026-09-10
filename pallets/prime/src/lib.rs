//! # Prime
//!
//! A downgraded sudo privilege that only allows cancelling or killing
//! referenda and rejecting treasury spends. It also carries the runtime
//! upgrade and registrar retirement calls, whose origins the runtime picks.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub mod weights;
pub use weights::*;

use core::marker::PhantomData;
use frame_support::traits::EnsureOrigin;
use frame_system::RawOrigin;
use pallet_identity::RegistrarIndex;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use alloc::vec::Vec;
	use frame_support::{dispatch::DispatchClass, pallet_prelude::*};
	use frame_system::{pallet_prelude::*, WeightInfo as _};

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config:
		frame_system::Config<RuntimeEvent: From<Event<Self>>> + pallet_identity::Config
	{
		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;

		/// Origin allowed to replace the runtime code.
		type UpgradeOrigin: EnsureOrigin<Self::RuntimeOrigin>;

		/// Origin allowed to retire a registrar.
		type RetireOrigin: EnsureOrigin<Self::RuntimeOrigin>;
	}

	/// Account holding the prime privileges.
	#[pallet::storage]
	pub type Key<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

	#[pallet::genesis_config]
	#[derive(frame_support::DefaultNoBound)]
	pub struct GenesisConfig<T: Config> {
		pub key: Option<T::AccountId>,
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			if let Some(key) = &self.key {
				Key::<T>::put(key);
			}
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// The prime key moved to a new account.
		KeyChanged { old: T::AccountId, new: T::AccountId },
		/// A registrar lost the power to judge identities.
		RegistrarRemoved { index: RegistrarIndex },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The caller is not the prime key.
		RequirePrime,
		/// No registrar holds this index.
		NoRegistrar,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Replace the runtime code, forwarding to `System::set_code` as root.
		#[pallet::call_index(0)]
		#[pallet::weight((
			<T as Config>::WeightInfo::upgrade()
				.saturating_add(<T as frame_system::Config>::SystemWeightInfo::set_code()),
			DispatchClass::Operational,
		))]
		pub fn upgrade(origin: OriginFor<T>, code: Vec<u8>) -> DispatchResultWithPostInfo {
			T::UpgradeOrigin::ensure_origin(origin)?;
			frame_system::Pallet::<T>::set_code(frame_system::RawOrigin::Root.into(), code)
		}

		/// Hand the prime key to a new account.
		#[pallet::call_index(1)]
		#[pallet::weight(<T as Config>::WeightInfo::set_key())]
		pub fn set_key(origin: OriginFor<T>, new: T::AccountId) -> DispatchResult {
			let old = Self::ensure_prime(origin)?;
			Key::<T>::put(&new);
			Self::deposit_event(Event::KeyChanged { old, new });
			Ok(())
		}

		/// Retire a registrar. Judgements it already gave stay in place. The
		/// index it held is never reused.
		#[pallet::call_index(2)]
		#[pallet::weight(<T as Config>::WeightInfo::remove_registrar())]
		pub fn remove_registrar(origin: OriginFor<T>, index: RegistrarIndex) -> DispatchResult {
			T::RetireOrigin::ensure_origin(origin)?;

			pallet_identity::Registrars::<T>::try_mutate(|registrars| -> DispatchResult {
				let seat = registrars
					.get_mut(index as usize)
					.ok_or(Error::<T>::NoRegistrar)?;
				ensure!(seat.is_some(), Error::<T>::NoRegistrar);
				*seat = None;
				Ok(())
			})?;

			Self::deposit_event(Event::RegistrarRemoved { index });
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Require a signed origin matching the stored prime key.
		pub(crate) fn ensure_prime(origin: OriginFor<T>) -> Result<T::AccountId, DispatchError> {
			let who = ensure_signed(origin)?;
			ensure!(Key::<T>::get().as_ref() == Some(&who), Error::<T>::RequirePrime);
			Ok(who)
		}
	}
}

/// Ensure the origin is a signed account matching the stored prime key.
pub struct EnsurePrime<T>(PhantomData<T>);

impl<T: Config> EnsureOrigin<T::RuntimeOrigin> for EnsurePrime<T> {
	type Success = T::AccountId;

	fn try_origin(o: T::RuntimeOrigin) -> Result<Self::Success, T::RuntimeOrigin> {
		o.into().and_then(|raw| match raw {
			RawOrigin::Signed(who) if Key::<T>::get().as_ref() == Some(&who) => Ok(who),
			raw => Err(T::RuntimeOrigin::from(raw)),
		})
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin() -> Result<T::RuntimeOrigin, ()> {
		let key = Key::<T>::get().ok_or(())?;
		Ok(RawOrigin::Signed(key).into())
	}
}
