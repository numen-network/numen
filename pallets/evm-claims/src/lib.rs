//! # EVM claims
//!
//! Frontier kept the balance and nonce of an H160 at `blake2_256("evm:" ++
//! h160)`, an account nobody holds a key for. pallet-revive resolves the same
//! H160 to `h160 ++ 0xEE * 12`. A claim moves the balance, the vesting
//! schedules and the nonce across.
//!
//! Funds can only land on the account the H160's own key or code controls. A
//! claim therefore needs no signature. Anyone may submit one, for any H160
//! that still has something to move. A claim pays its fee out of the funds it
//! moves, so a balance that does not cover the fee stays unclaimable. A nonce
//! moves for free, since nobody can manufacture one.

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
	use super::*;
	use frame_support::{
		pallet_prelude::*,
		traits::{
			fungible::{Balanced, Credit, Inspect, Mutate, Unbalanced},
			tokens::{Fortitude, Precision, Preservation},
			LockIdentifier, OnUnbalanced, VestingSchedule,
		},
	};
	use frame_system::{ensure_authorized, pallet_prelude::*};
	use sp_core::H160;
	use sp_runtime::{
		traits::{BlakeTwo256, BlockNumberProvider, Hash, Zero},
		AccountId32,
	};

	type Balances<T> = pallet_balances::Pallet<T>;
	type Vesting<T> = pallet_vesting::Pallet<T>;

	const VESTING_ID: LockIdentifier = *b"vesting ";

	/// Custom validity error of a claim with no nonce to move and funds worth
	/// no more than its fee.
	pub const NOTHING_TO_CLAIM: u8 = 0;

	/// Custom validity error of a claim whose vesting schedules do not fit
	/// next to the ones the new account already has.
	pub const NO_ROOM_FOR_VESTING: u8 = 1;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config:
		frame_system::Config<AccountId = AccountId32, RuntimeEvent: From<Event<Self>>>
		+ pallet_balances::Config
		+ pallet_vesting::Config<Currency = pallet_balances::Pallet<Self>>
	{
		/// What a claim costs, taken out of the funds it moves.
		type ClaimFee: Get<Self::Balance>;

		/// Where the fee goes.
		type OnClaimFee: OnUnbalanced<Credit<Self::AccountId, pallet_balances::Pallet<Self>>>;

		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// `account` took over `amount` and the nonce the old account of
		/// `address` reached.
		Claimed { address: H160, account: T::AccountId, amount: T::Balance, nonce: T::Nonce },
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Moves what the Frontier account of `address` holds to the account
		/// pallet-revive resolves `address` to, and pays the fee from it.
		#[pallet::call_index(0)]
		#[pallet::weight(<T as Config>::WeightInfo::claim(T::MAX_VESTING_SCHEDULES))]
		#[pallet::weight_of_authorize(<T as Config>::WeightInfo::authorize_claim())]
		#[pallet::authorize(Self::authorize_claim)]
		pub fn claim(origin: OriginFor<T>, address: H160) -> DispatchResultWithPostInfo {
			ensure_authorized(origin)?;
			let legacy = Self::legacy_account(&address);
			let fee = Self::fee_due(&legacy);
			let schedules = Self::schedules(&legacy);
			Self::move_legacy(address)?;
			Self::pay_fee(&Self::revive_account(&address), fee)?;
			Ok(Some(<T as Config>::WeightInfo::claim(schedules)).into())
		}
	}

	impl<T: Config> Pallet<T> {
		fn authorize_claim(
			_source: TransactionSource,
			address: &H160,
		) -> TransactionValidityWithRefund {
			let legacy = Self::legacy_account(address);
			let account = Self::revive_account(address);
			let nonce_due = frame_system::Pallet::<T>::account_nonce(&legacy) >
				frame_system::Pallet::<T>::account_nonce(&account);
			let funds_due =
				Self::movable(&legacy) && Self::spendable(&legacy) > T::ClaimFee::get();
			if !(nonce_due || funds_due) {
				return Err(InvalidTransaction::Custom(NOTHING_TO_CLAIM).into());
			}
			if Self::schedules(&legacy) + Self::schedules(&account) > T::MAX_VESTING_SCHEDULES {
				return Err(InvalidTransaction::Custom(NO_ROOM_FOR_VESTING).into());
			}
			ValidTransaction::with_tag_prefix("EvmClaims")
				.and_provides(address)
				.propagate(true)
				.build()
				.map(|validity| (validity, Weight::zero()))
		}

		/// The fee a claim takes, no more than the old account can spend.
		fn fee_due(legacy: &T::AccountId) -> T::Balance {
			if Self::movable(legacy) {
				T::ClaimFee::get().min(Self::spendable(legacy))
			} else {
				Zero::zero()
			}
		}

		/// The balance of `who` that is neither locked nor keeping it alive.
		fn spendable(who: &T::AccountId) -> T::Balance {
			Balances::<T>::reducible_balance(who, Preservation::Expendable, Fortitude::Polite)
		}

		fn schedules(who: &T::AccountId) -> u32 {
			pallet_vesting::Vesting::<T>::decode_len(who).unwrap_or(0) as u32
		}

		fn pay_fee(account: &T::AccountId, fee: T::Balance) -> DispatchResult {
			if fee.is_zero() {
				return Ok(());
			}
			let paid = Balances::<T>::withdraw(
				account,
				fee,
				Precision::BestEffort,
				Preservation::Preserve,
				Fortitude::Polite,
			)?;
			T::OnClaimFee::on_unbalanced(paid);
			Ok(())
		}

		/// The account Frontier kept the balance and nonce of `address` at.
		pub fn legacy_account(address: &H160) -> T::AccountId {
			let mut preimage = [0u8; 24];
			preimage[..4].copy_from_slice(b"evm:");
			preimage[4..].copy_from_slice(address.as_bytes());
			AccountId32::new(BlakeTwo256::hash(&preimage).0)
		}

		/// The account pallet-revive resolves `address` to.
		pub fn revive_account(address: &H160) -> T::AccountId {
			let mut account = [0xEE; 32];
			account[..20].copy_from_slice(address.as_bytes());
			AccountId32::new(account)
		}

		/// Whether the old account of `address` still holds anything to move.
		pub fn claimable(address: &H160) -> bool {
			let legacy = Self::legacy_account(address);
			let nonce = frame_system::Pallet::<T>::account_nonce(&legacy);
			(Self::movable(&legacy) && !Balances::<T>::balance(&legacy).is_zero()) ||
				nonce > frame_system::Pallet::<T>::account_nonce(&Self::revive_account(address))
		}

		/// Moves the balance, vesting schedules and nonce of the old account of
		/// `address` and drops the old account once nothing is left on it.
		/// Balances under a hold, a reserve, a freeze or a lock other than
		/// vesting stay where they are. Only the nonce moves then.
		pub fn move_legacy(address: H160) -> DispatchResult {
			let legacy = Self::legacy_account(&address);
			let account = Self::revive_account(&address);
			let nonce = Self::carry_nonce(&legacy, &account);
			let amount = if Self::movable(&legacy) {
				Self::move_balance(&legacy, &account)?
			} else {
				Zero::zero()
			};
			let left = frame_system::Account::<T>::get(&legacy);
			if left.consumers.is_zero() && Balances::<T>::total_balance(&legacy).is_zero() {
				frame_system::Account::<T>::remove(&legacy);
			}
			Self::deposit_event(Event::Claimed { address, account, amount, nonce });
			Ok(())
		}

		/// Lifts the nonce of an Ethereum signer that has not signed under
		/// pallet-revive yet to the one its H160 reached under Frontier. None
		/// of the transactions Frontier executed for it can then run again.
		pub fn lift_nonce(who: &T::AccountId) {
			let bytes: &[u8; 32] = who.as_ref();
			if bytes[20..] != [0xEE; 12] || !frame_system::Pallet::<T>::account_nonce(who).is_zero()
			{
				return;
			}
			Self::carry_nonce(&Self::legacy_account(&H160::from_slice(&bytes[..20])), who);
		}

		/// Raises the nonce of `account` to that of `legacy`. An account whose
		/// nonce leaves zero gets the provider a first signature gives. That
		/// keeps it and its nonce through a drain.
		fn carry_nonce(legacy: &T::AccountId, account: &T::AccountId) -> T::Nonce {
			let nonce = frame_system::Pallet::<T>::account_nonce(legacy);
			let current = frame_system::Pallet::<T>::account_nonce(account);
			if nonce <= current {
				return current;
			}
			if current.is_zero() {
				frame_system::Pallet::<T>::inc_providers(account);
			}
			frame_system::Account::<T>::mutate(account, |info| info.nonce = nonce);
			nonce
		}

		fn movable(legacy: &T::AccountId) -> bool {
			Balances::<T>::reserved_balance(legacy).is_zero() &&
				pallet_balances::Freezes::<T>::get(legacy).is_empty() &&
				pallet_balances::Locks::<T>::get(legacy).iter().all(|lock| lock.id == VESTING_ID)
		}

		/// Moves the free balance with its vesting schedules. An empty `account`
		/// first gets an inactive existential deposit, so what it can spend
		/// afterwards is exactly what `legacy` held.
		fn move_balance(
			legacy: &T::AccountId,
			account: &T::AccountId,
		) -> Result<T::Balance, DispatchError> {
			let amount = Balances::<T>::balance(legacy);
			if amount.is_zero() {
				return Ok(amount);
			}
			let now = <T as pallet_vesting::Config>::BlockNumberProvider::current_block_number();
			let schedules = pallet_vesting::Vesting::<T>::get(legacy).unwrap_or_default();
			while pallet_vesting::Vesting::<T>::contains_key(legacy) {
				Vesting::<T>::remove_vesting_schedule(legacy, 0)?;
			}
			if Balances::<T>::total_balance(account).is_zero() {
				let deposit = Balances::<T>::minimum_balance();
				Balances::<T>::mint_into(account, deposit)?;
				Balances::<T>::deactivate(deposit);
			}
			Balances::<T>::transfer(legacy, account, amount, Preservation::Expendable)?;
			for schedule in schedules {
				if schedule.locked_at::<T::BlockNumberToBalance>(now).is_zero() {
					continue;
				}
				Vesting::<T>::add_vesting_schedule(
					account,
					schedule.locked(),
					schedule.per_block(),
					schedule.starting_block(),
				)?;
			}
			Ok(amount)
		}
	}
}
