//! Identity registrars seated at genesis. `pallet-identity` has no genesis
//! config, so this pallet writes its `Registrars` storage instead.

use crate::Runtime;

#[frame_support::pallet]
pub mod pallet_genesis_registrars {
	use alloc::vec::Vec;
	use frame_support::pallet_prelude::*;
	use pallet_identity::{RegistrarInfo, Registrars};
	use sp_runtime::traits::Zero;

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_identity::Config {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::genesis_config]
	#[derive(frame_support::DefaultNoBound)]
	pub struct GenesisConfig<T: Config> {
		/// Registrar accounts in index order, each charging no fee.
		pub registrars: Vec<T::AccountId>,
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			for account in &self.registrars {
				let info = RegistrarInfo {
					account: account.clone(),
					fee: Zero::zero(),
					fields: Default::default(),
				};
				Registrars::<T>::try_append(Some(info)).expect("genesis registrars fit MaxRegistrars");
			}
		}
	}
}

impl pallet_genesis_registrars::Config for Runtime {}
