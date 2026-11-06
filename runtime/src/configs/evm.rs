//! EVM configuration.

use crate::Runtime;

/// EVM slot the native balance ERC20 facade is mounted at.
const BALANCES_ERC20: u16 = 0x802;

/// EVM facts the chain publishes for off chain readers.
#[frame_support::pallet]
pub mod pallet_precompiles {
	use super::BALANCES_ERC20;
	use sp_core::H160;

	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::extra_constants]
	impl<T: Config> Pallet<T> {
		/// EVM address of the native balance ERC20 facade. A wallet builds
		/// its withdraw call against this address and an indexer excludes
		/// it from token listings, so it is published for both to read
		/// rather than copied into each.
		#[pallet::constant_name(BalancesErc20)]
		pub fn balances_erc20() -> H160 {
			let mut address = H160::zero();
			address.0[18..].copy_from_slice(&BALANCES_ERC20.to_be_bytes());
			address
		}
	}
}

impl pallet_precompiles::Config for Runtime {}
