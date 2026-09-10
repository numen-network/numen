//! Block reward pallet.
//!
//! Resolves the block author (miner) through `FindAuthor`, then mints a halving
//! reward to their account on each block. The reward starts at `InitialReward`
//! and halves every `HalvingInterval` blocks, so total mined issuance is the
//! geometric sum `2 * InitialReward * HalvingInterval`, capping emission
//! without any on-chain supply check.
//!
//! Orphan and uncle blocks receive no reward because their state changes
//! are never applied to the canonical chain.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub mod weights;
pub use weights::*;

#[frame_support::pallet]
pub mod pallet {
    use crate::weights::WeightInfo;
    use frame_support::{
        pallet_prelude::*,
        traits::{Currency, FindAuthor},
    };
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{One, Zero};

    type BalanceOf<T> =
        <<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The currency used to mint block rewards.
        type Currency: Currency<Self::AccountId>;

        /// Resolves the block author from the pre-runtime digest.
        type FindAuthor: FindAuthor<Self::AccountId>;

        /// Reward for the first halving period (in smallest units).
        #[pallet::constant]
        type InitialReward: Get<BalanceOf<Self>>;

        /// Block count between reward halvings.
        #[pallet::constant]
        type HalvingInterval: Get<BlockNumberFor<Self>>;

        /// Weight information for this pallet's block hook.
        type WeightInfo: WeightInfo;
    }

    const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    pub struct Pallet<T>(_);

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        // `on_finalize` has no way to report what it spent, so the author
        // payout is paid for up front here.
        fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
            T::WeightInfo::on_finalize()
        }

        fn on_finalize(n: BlockNumberFor<T>) {
            if let Some(author) = Self::find_author() {
                let reward = Self::block_reward(n);
                if !reward.is_zero() {
                    let _ = T::Currency::deposit_creating(&author, reward);
                }
            }
        }
    }

    impl<T: Config> Pallet<T> {
        /// Reward at height `n`, which is `InitialReward` halved once per
        /// elapsed `HalvingInterval`. Integer division truncates each halving
        /// and the reward reaches zero once fully shifted out, ending emission.
        fn block_reward(n: BlockNumberFor<T>) -> BalanceOf<T> {
            let mut halvings = n / T::HalvingInterval::get();
            let mut reward = T::InitialReward::get();
            let two = BalanceOf::<T>::from(2u32);
            while !halvings.is_zero() && !reward.is_zero() {
                reward /= two;
                halvings -= One::one();
            }
            reward
        }

        /// Extract the block author from the PoW pre-runtime digest.
        ///
        /// The miner encodes their `AccountId` as the payload of a
        /// `PreRuntime(POW_ENGINE_ID, _)` digest item.
        fn find_author() -> Option<T::AccountId> {
            let digest = frame_system::Pallet::<T>::digest();
            T::FindAuthor::find_author(digest.logs.iter().filter_map(|log| log.as_pre_runtime()))
        }
    }
}
