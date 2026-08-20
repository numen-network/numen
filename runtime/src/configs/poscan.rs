//! PoScan facts the chain publishes.
//!
//! Seal verification lives in a runtime API rather than a pallet, so the scan
//! has no on chain home of its own. This is that home. It holds no state and
//! no calls, only what an off chain reader needs to tell which PoScan this
//! chain runs.

use crate::Runtime;

#[frame_support::pallet]
pub mod pallet_poscan {
	use alloc::vec::Vec;

	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::extra_constants]
	impl<T: Config> Pallet<T> {
		/// Domain separation prefix of the scan the runtime verifies against.
		/// An external miner reads it to confirm it speaks this protocol
		/// before it starts hashing.
		#[pallet::constant_name(Protocol)]
		fn protocol() -> Vec<u8> {
			::poscan::POSCAN_PROTOCOL.to_vec()
		}

		/// Consensus engine the PoW digest is tagged with. An indexer reads it
		/// to pick the seal out of a block's digest logs.
		#[pallet::constant_name(Engine)]
		fn engine() -> [u8; 4] {
			sp_consensus_pow::POW_ENGINE_ID
		}
	}
}

impl pallet_poscan::Config for Runtime {}
