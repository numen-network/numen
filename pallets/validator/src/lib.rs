//! # Pallet Validator
//!
//! Manages the full lifecycle of validators: lock, auto-renewal, exit,
//! and kick.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub mod weights;
pub use weights::*;

extern crate alloc;

use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use frame_support::{
    traits::{Currency, Get, LockableCurrency},
    BoundedVec,
};
use scale_info::TypeInfo;
use sp_runtime::traits::{Saturating, Zero};

/// Runtime adapter that lets `pallet-validator` query the session-key registry
/// without taking a hard dependency on `pallet-session`.
///
/// In production runtimes this is implemented over `pallet-session`; tests
/// use the no-op `()` implementation below.
pub trait SessionInterface<AccountId> {
    /// Whether `who` has session keys registered. Required so a candidate
    /// cannot be promoted to the active set without an authoring identity.
    fn has_keys(who: &AccountId) -> bool;
}

impl<AccountId> SessionInterface<AccountId> for () {
    fn has_keys(_who: &AccountId) -> bool {
        true
    }
}

/// Benchmark-only setup hook.
///
/// Both gates guarding `lock` live outside this pallet, so a generic benchmark
/// cannot walk a candidate past them. The runtime owns that setup, which also
/// makes the benchmark pay the real cost of both lookups.
#[cfg(feature = "runtime-benchmarks")]
pub trait BenchmarkHelper<AccountId> {
    fn make_eligible(who: &AccountId);
}

#[cfg(feature = "runtime-benchmarks")]
impl<AccountId> BenchmarkHelper<AccountId> for () {
    fn make_eligible(_who: &AccountId) {}
}

/// Balance type alias derived from the configured `Currency`.
pub type BalanceOf<T> = <<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

/// Lifecycle state of a validator's stake.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Encode, Decode, DecodeWithMemTracking, TypeInfo, MaxEncodedLen)]
pub enum ValidatorStatus {
    /// Active in the validator set; eligible for auto-renewal.
    Active,
    /// On the way out of the set, either by voluntary exit or by being
    /// dropped before promotion. Auto-renewal stopped, awaiting expiry.
    ExitRequested,
    /// Removed from the active set due to offline or equivocation.
    /// The specific reason is conveyed by the [`pallet::KickReason`] event.
    Kicked,
}

/// Lock record for a validator's stake.
#[derive(
    Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeWithMemTracking, TypeInfo, MaxEncodedLen,
)]
pub struct LockInfo<Balance, BlockNumber> {
    pub amount: Balance,
    pub lock_block: BlockNumber,
    pub expiry_block: BlockNumber,
    pub status: ValidatorStatus,
}

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use frame_support::{
        pallet_prelude::*,
        traits::{Contains, Defensive, LockIdentifier, WithdrawReasons},
    };
    use frame_system::pallet_prelude::*;

    const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config<RuntimeEvent: From<Event<Self>>> {
        /// Currency used for validator stake locking.
        type Currency: LockableCurrency<Self::AccountId, Moment = BlockNumberFor<Self>>;

        /// Adapter that bridges to the runtime's session-key registry.
        type SessionInterface: SessionInterface<Self::AccountId>;

        /// Session period used for periodic session rotation.
        #[pallet::constant]
        type SessionPeriod: Get<BlockNumberFor<Self>>;

        /// Delay before the first session starts.
        #[pallet::constant]
        type SessionOffset: Get<BlockNumberFor<Self>>;

        /// Amount to lock when registering as a validator.
        #[pallet::constant]
        type LockAmount: Get<BalanceOf<Self>>;

        /// Origin allowed to manage [`StakeExemptAccounts`].
        type ExemptOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Identity requirement every candidate must pass to join via `lock`.
        /// Stake exemption does not bypass it. Genesis appointment does, along
        /// with every other join condition.
        type IdentityGate: Contains<Self::AccountId>;

        /// Lock duration applied at registration and on each renewal.
        #[pallet::constant]
        type LockDuration: Get<BlockNumberFor<Self>>;

        /// Lock identifier used when calling set_lock on the underlying currency.
        #[pallet::constant]
        type LockId: Get<LockIdentifier>;

        /// Upper bound for the number of pending/active validators tracked in storage.
        #[pallet::constant]
        type MaxValidators: Get<u32>;

        /// Interval between auto-renewal sweeps.
        #[pallet::constant]
        type RenewInterval: Get<BlockNumberFor<Self>>;

        /// Number of consecutive offline sessions that triggers an offline kick.
        #[pallet::constant]
        type OfflineThreshold: Get<u32>;

        /// Blocks a kicked validator must wait before staking again.
        #[pallet::constant]
        type RejoinCooldownPeriod: Get<BlockNumberFor<Self>>;

        /// Weight information for extrinsics in this pallet.
        type WeightInfo: WeightInfo;

        /// Setup hook that walks a benchmark account past the identity gate
        /// and the session key check.
        #[cfg(feature = "runtime-benchmarks")]
        type BenchmarkHelper: BenchmarkHelper<Self::AccountId>;
    }

    /// Active validator lock records, keyed by account.
    #[pallet::storage]
    pub type ValidatorLocks<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        LockInfo<BalanceOf<T>, BlockNumberFor<T>>,
        OptionQuery,
    >;

    /// Validators waiting to be promoted into the active set at the next session boundary.
    #[pallet::storage]
	pub type PendingValidators<T: Config> = StorageValue<_, BoundedVec<T::AccountId, T::MaxValidators>, ValueQuery>;

    /// Validators currently selected for the active session. Updated at every session boundary.
    #[pallet::storage]
	pub type DesiredValidators<T: Config> = StorageValue<_, BoundedVec<T::AccountId, T::MaxValidators>, ValueQuery>;

    /// Accounts allowed to join the validator set without locking stake.
    /// Consulted only when a candidate joins via `lock`; validators already
    /// in the set keep the amount recorded at join time.
    #[pallet::storage]
	pub type StakeExemptAccounts<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

    /// Rejoin cooldown deadline per account (block number).
    #[pallet::storage]
	pub type RejoinCooldown<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>, OptionQuery>;

    /// Consecutive offline session count per account.
    #[pallet::storage]
	pub type OfflineSessionCount<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Accounts reported as offline during the current session. Cleared when the next session boundary is processed.
    #[pallet::storage]
	pub type OfflineThisSession<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

    /// Accounts reported for GRANDPA equivocation during the current session. Cleared when the next session boundary is processed.
    #[pallet::storage]
	pub type EquivocationThisSession<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

    /// Genesis configuration for `pallet-validator`.
    ///
    /// `initial_validators` lists the accounts that must be present in the
    /// active validator set at block 0. They are appointed by the chain spec
    /// and stake nothing. Each account still gets a lock record with a zero
    /// amount so auto-renewal, exit, and kick logic operates on real records.
    /// Each account is also seeded into [`StakeExemptAccounts`] so it can
    /// rejoin without stake until the exempt origin removes it.
    /// The accounts are pushed directly into `DesiredValidators` (not
    /// `PendingValidators`) so that the very first session already has a
    /// non-empty authority set for `pallet-session` and downstream consumers
    /// such as `pallet-grandpa` and `pallet-im-online`.
    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub initial_validators: alloc::vec::Vec<T::AccountId>,
        #[serde(skip)]
        pub _phantom: core::marker::PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let duration = T::LockDuration::get();
            let now = BlockNumberFor::<T>::zero();
            let expiry_block = now.saturating_add(duration);

            let mut active: BoundedVec<T::AccountId, T::MaxValidators> = BoundedVec::default();
            for who in &self.initial_validators {
                if ValidatorLocks::<T>::contains_key(who) {
                    continue;
                }
                ValidatorLocks::<T>::insert(
                    who,
                    LockInfo {
                        amount: BalanceOf::<T>::zero(),
                        lock_block: now,
                        expiry_block,
                        status: ValidatorStatus::Active,
                    },
                );
                StakeExemptAccounts::<T>::insert(who, ());
                active
                    .try_push(who.clone())
                    .expect("Genesis validators exceed MaxValidators");
            }
            DesiredValidators::<T>::put(active);
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A validator locked stake. `[who, amount, expiry_block]`
        ValidatorLocked {
            who: T::AccountId,
            amount: BalanceOf<T>,
            expiry_block: BlockNumberFor<T>,
        },
        /// A validator requested voluntary exit.
        ValidatorExitRequested { who: T::AccountId },
        /// A validator was kicked (offline or equivocation).
		ValidatorKicked { who: T::AccountId, reason: KickReason },
        /// A validator's lock was released after expiry.
		LockReleased { who: T::AccountId, amount: BalanceOf<T> },
        /// A GRANDPA equivocation report was processed for an active validator.
        EquivocationReported { who: T::AccountId },
        /// A pending validator was dropped before promotion. Its stake stays
        /// locked until expiry.
        PendingValidatorDropped { who: T::AccountId },
        /// An account's stake exemption was granted or revoked.
        StakeExemptSet { who: T::AccountId, exempt: bool },
    }

    /// Reason a validator was removed from the active set.
    #[derive(
        Clone,
        Copy,
        PartialEq,
        Eq,
        Debug,
        Encode,
        Decode,
        DecodeWithMemTracking,
        TypeInfo,
        MaxEncodedLen,
    )]
    pub enum KickReason {
        /// Removed for being offline beyond the threshold.
        Offline,
        /// Removed for GRANDPA equivocation.
        Equivocation,
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Account is already a validator.
        AlreadyValidator,
        /// Account is not a registered validator.
        NotValidator,
        /// Operation not permitted in the current validator status.
        InvalidStatus,
        /// Lock has not yet reached its expiry block.
        LockNotExpired,
        /// Account is currently within a rejoin cooldown after being kicked.
        InCooldown,
        /// Account does not have enough free balance to cover the configured lock.
        InsufficientBalance,
        /// The pending validator queue is full.
        TooManyValidators,
        /// Caller has not registered session keys.
        SessionKeysNotRegistered,
        /// Caller does not pass the configured identity gate.
        IdentityNotQualified,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let duration = T::LockDuration::get();
            let interval = T::RenewInterval::get();

            // First pass: collect expired locks and (when enabled) renewal candidates.
            let mut scanned = 0u32;
            let mut to_release: alloc::vec::Vec<(T::AccountId, BalanceOf<T>)> =
                alloc::vec::Vec::new();
            let mut to_renew: alloc::vec::Vec<T::AccountId> = alloc::vec::Vec::new();
            for (who, info) in ValidatorLocks::<T>::iter() {
                scanned.saturating_inc();
                if info.expiry_block <= now {
                    to_release.push((who, info.amount));
                    continue;
                }
                if interval.is_zero() || info.status != ValidatorStatus::Active {
                    continue;
                }
                let remaining = info.expiry_block.saturating_sub(now);
                let elapsed_window = duration.saturating_sub(remaining);
                if elapsed_window >= interval {
                    to_renew.push(who);
                }
            }

            // Release expired locks: drop the currency lock, clear storage, emit event.
            // Also clear stale liveness-tracking entries keyed by the account.
            // `RejoinCooldown` is intentionally preserved: it represents a
            // post-release penalty that must outlive the underlying lock.
            for (who, amount) in to_release {
                T::Currency::remove_lock(T::LockId::get(), &who);
                ValidatorLocks::<T>::remove(&who);
                Self::deposit_event(Event::LockReleased { who, amount });
            }

            // Renew Active locks whose elapsed window has reached the configured interval.
            for who in to_renew {
                ValidatorLocks::<T>::mutate(&who, |maybe_info| {
                    if let Some(info) = maybe_info {
                        info.expiry_block = now.saturating_add(duration);
                        Self::deposit_event(Event::ValidatorLocked {
                            who: who.clone(),
                            amount: info.amount,
                            expiry_block: info.expiry_block,
                        });
                    }
                });
            }

            // Billed on the scan length, not on the mutation count. The sweep
            // reads every lock even when nothing expires, and the benchmark
            // measures the release path, so a pure scan is covered too.
            T::WeightInfo::on_initialize(scanned)
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Lock the configured stake amount for the configured duration and
        /// queue the caller into [`PendingValidators`] for the next session.
        ///
        /// The locked amount and duration are taken from `Config::LockAmount`
        /// and `Config::LockDuration` respectively; callers do not choose them.
        ///
        /// A caller whose previous lock has not expired yet may rejoin, which
        /// refreshes that lock instead of placing a second one. The refresh
        /// only ever tightens it, so rejoining never returns stake early.
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::lock())]
        pub fn lock(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let active = DesiredValidators::<T>::get();
            let pending = PendingValidators::<T>::get();
            ensure!(
                !active.iter().any(|a| a == &who) && !pending.iter().any(|a| a == &who),
                Error::<T>::AlreadyValidator,
            );

            let active_len = active.len() as u32;
            let pending_len = pending.len() as u32;
            ensure!(
                active_len.saturating_add(pending_len) < T::MaxValidators::get(),
                Error::<T>::TooManyValidators,
            );

            ensure!(
                T::SessionInterface::has_keys(&who),
                Error::<T>::SessionKeysNotRegistered,
            );

            ensure!(
                T::IdentityGate::contains(&who),
                Error::<T>::IdentityNotQualified,
            );

            let now = frame_system::Pallet::<T>::block_number();
            if let Some(deadline) = RejoinCooldown::<T>::get(&who) {
                if deadline >= now {
                    return Err(Error::<T>::InCooldown.into());
                }
                RejoinCooldown::<T>::remove(&who);
            }

            let required = Self::required_lock(&who);
            let fresh_expiry = now.saturating_add(T::LockDuration::get());

            // Relocking refreshes an existing commitment and a refresh must
            // never weaken it, so both fields keep whichever value binds
            // harder. Without this, a zero required amount frees the stake
            // outright, because `set_lock` treats zero as removal.
            let (amount, expiry_block) = match ValidatorLocks::<T>::get(&who) {
                Some(previous) => (
                    previous.amount.max(required),
                    previous.expiry_block.max(fresh_expiry),
                ),
                None => (required, fresh_expiry),
            };

            ensure!(
                T::Currency::free_balance(&who) >= amount,
                Error::<T>::InsufficientBalance,
            );

            PendingValidators::<T>::mutate(|queue| {
                let _ = queue
                    .try_push(who.clone())
                    .defensive_proof("preflight ensures active+pending < MaxValidators");
            });

            T::Currency::set_lock(
                T::LockId::get(),
                &who,
                amount,
                WithdrawReasons::all(),
            );

            ValidatorLocks::<T>::insert(
                &who,
                LockInfo {
                    amount,
                    lock_block: now,
                    expiry_block,
                    status: ValidatorStatus::Active,
                },
            );

            Self::deposit_event(Event::ValidatorLocked {
                who,
                amount,
                expiry_block,
            });
            Ok(())
        }

        /// Request voluntary exit from the active validator set.
        ///
        /// Only an `Active` validator may call this. The validator's status
        /// becomes `ExitRequested`, auto-renewal stops, and the account is
        /// removed from [`PendingValidators`] so it will not be promoted at
        /// the next session boundary. The underlying currency lock is kept in
        /// place until its original `expiry_block` is reached; early unlocking
        /// is not permitted.
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::request_exit())]
        pub fn request_exit(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            ValidatorLocks::<T>::try_mutate(&who, |maybe_info| -> DispatchResult {
                let info = maybe_info.as_mut().ok_or(Error::<T>::NotValidator)?;
                ensure!(
                    info.status == ValidatorStatus::Active,
                    Error::<T>::InvalidStatus
                );
                info.status = ValidatorStatus::ExitRequested;
                Ok(())
            })?;

            PendingValidators::<T>::mutate(|queue| {
                if let Some(pos) = queue.iter().position(|a| a == &who) {
                    queue.remove(pos);
                }
            });

            Self::deposit_event(Event::ValidatorExitRequested { who });
            Ok(())
        }

        /// Grant or revoke stake exemption for `who`.
        ///
        /// Idempotent so the managing origin never has to read state first.
        /// Revoking does not touch validators already in the set; their lock
        /// amount was fixed when they joined.
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::set_stake_exempt())]
        pub fn set_stake_exempt(
            origin: OriginFor<T>,
            who: T::AccountId,
            exempt: bool,
        ) -> DispatchResult {
            T::ExemptOrigin::ensure_origin(origin)?;
            if exempt {
                StakeExemptAccounts::<T>::insert(&who, ());
            } else {
                StakeExemptAccounts::<T>::remove(&who);
            }
            Self::deposit_event(Event::StakeExemptSet { who, exempt });
            Ok(())
        }
    }
}

/// Log target for liveness-related diagnostics.
const LOG_TARGET: &str = "runtime::validator";

impl<T: Config> Pallet<T> {
    /// Fresh stake `who` must put up to join the validator set. Zero for an
    /// exempt account, which places no currency lock at all because `set_lock`
    /// treats zero as removal. It is a floor, not the final amount, since a
    /// rejoining account keeps whatever its unexpired lock already holds.
    fn required_lock(who: &T::AccountId) -> BalanceOf<T> {
        if StakeExemptAccounts::<T>::contains_key(who) {
            Zero::zero()
        } else {
            T::LockAmount::get()
        }
    }

    /// Record `who` as offline for the current session.
    ///
    /// Intended to be invoked by the runtime adapter that bridges
    /// `pallet-im-online`'s `ReportUnresponsiveness` into this pallet.
    /// Repeated calls within the same session are idempotent.
    pub fn note_offline(who: &T::AccountId) {
        if !ValidatorLocks::<T>::contains_key(who) {
            return;
        }
        OfflineThisSession::<T>::insert(who, ());
        log::debug!(
            target: LOG_TARGET,
            "validator reported offline for current session",
        );
    }

    /// Record `who` for a GRANDPA equivocation kick at the next session
    /// boundary.
    ///
    /// Mirrors the offline path: rather than mutating validator state in the
    /// middle of a session — which would flip the status to `Kicked` while the
    /// account is still a member of the active set until the boundary — this
    /// only flags the account. The actual kick (status transition, rejoin
    /// cooldown, removal from the pending queue and active set) is applied
    /// atomically in `new_session` via `process_equivocations`.
    ///
    /// Idempotent: non-validators, accounts not in [`ValidatorStatus::Active`],
    /// and repeated reports within the same session are silently ignored. An
    /// `EquivocationReported` event is emitted immediately for observability.
    pub fn note_equivocation(who: &T::AccountId) {
        let is_active = ValidatorLocks::<T>::get(who)
            .map(|info| info.status == ValidatorStatus::Active)
            .unwrap_or(false);
        if !is_active {
            log::debug!(
                target: LOG_TARGET,
                "equivocation report ignored: account is not an active validator",
            );
            return;
        }
        if EquivocationThisSession::<T>::contains_key(who) {
            // Already flagged this session; keep repeated reports idempotent.
            return;
        }
        EquivocationThisSession::<T>::insert(who, ());
        Self::deposit_event(Event::EquivocationReported { who: who.clone() });
        log::info!(
            target: LOG_TARGET,
            "equivocation reported; kick deferred to next session boundary",
        );
    }

    /// Apply a kick to `who`: flip the lock status from `Active` to `Kicked`,
    /// start the rejoin cooldown, and clear any transient offline tracking
    /// state. The currency lock is left in place so funds unlock at the
    /// original `expiry_block`; auto-renewal stops because the status left
    /// `Active`. Emits `ValidatorKicked` with `reason`. Returns whether a kick
    /// actually happened (an account not in `Active` is left untouched).
    fn kick_validator(who: &T::AccountId, reason: KickReason) -> bool {
        let mut kicked = false;
        ValidatorLocks::<T>::mutate(who, |maybe_info| {
            if let Some(info) = maybe_info
                && info.status == ValidatorStatus::Active
            {
                info.status = ValidatorStatus::Kicked;
                kicked = true;
            }
        });
        if !kicked {
            return false;
        }
        let now = frame_system::Pallet::<T>::block_number();
        let cooldown = T::RejoinCooldownPeriod::get();
        RejoinCooldown::<T>::insert(who, now.saturating_add(cooldown));
        OfflineSessionCount::<T>::remove(who);
        OfflineThisSession::<T>::remove(who);
        Self::deposit_event(Event::ValidatorKicked {
            who: who.clone(),
            reason,
        });
        true
    }

    /// Apply deferred GRANDPA equivocation kicks at a session boundary.
    ///
    /// For each account flagged via `note_equivocation` during the session,
    /// flip the status to `Kicked`, record a rejoin cooldown, and emit
    /// `ValidatorKicked`. Performing this here keeps the status transition
    /// atomic with removal from the active set (done by `new_session`), so an
    /// equivocator is never simultaneously `Kicked` and still a member of
    /// `DesiredValidators`. Runs before `process_offline_counters` so an
    /// equivocation kick takes precedence over a concurrent offline kick for
    /// the same account.
    fn process_equivocations() {
        let flagged: alloc::vec::Vec<T::AccountId> =
            EquivocationThisSession::<T>::drain().map(|(who, _)| who).collect();
        for who in flagged {
            if Self::kick_validator(&who, KickReason::Equivocation) {
                log::info!(
                    target: LOG_TARGET,
                    "validator kicked due to GRANDPA equivocation",
                );
            }
        }
    }

    /// Update offline counters for the current active set and kick any
    /// validator that has reached `OfflineThreshold` consecutive offline
    /// sessions. Called at every session boundary before the active set is
    /// recomputed.
    fn process_offline_counters() {
        let threshold = T::OfflineThreshold::get();
        let active = DesiredValidators::<T>::get();

        for who in active.iter() {

            // Only Active validators participate in the offline accounting.
            // Anything else (ExitRequested / Kicked / Cooldown / removed) is
            // already on its way out and must not be overwritten by an offline
            // kick, which would also clobber the original exit/kick reason.
            let is_active = ValidatorLocks::<T>::get(who)
                .map(|info| info.status == ValidatorStatus::Active)
                .unwrap_or(false);
            if !is_active {
                OfflineSessionCount::<T>::remove(who);
                continue;
            }

            if OfflineThisSession::<T>::take(who).is_none() {
                if OfflineSessionCount::<T>::contains_key(who) {
                    OfflineSessionCount::<T>::remove(who);
                }
                continue;
            }

            let count = OfflineSessionCount::<T>::get(who).saturating_add(1);
            if count < threshold {
                OfflineSessionCount::<T>::insert(who, count);
                log::debug!(
                    target: LOG_TARGET,
                    "offline counter incremented to {} (threshold {})",
                    count,
                    threshold,
                );
                continue;
            }

            if Self::kick_validator(who, KickReason::Offline) {
                log::info!(
                    target: LOG_TARGET,
                    "validator kicked after {} consecutive offline sessions",
                    threshold,
                );
            }
        }

        // Drop any leftover entries (offenders that are not in the active set).
        let _ = OfflineThisSession::<T>::clear(u32::MAX, None);
    }
}

/// Drives `pallet-session` from the validator lifecycle storage.
///
/// At every new session, the active set is recomputed as:
/// * keep current `DesiredValidators` whose [`ValidatorLocks`] entry is still in
///   [`ValidatorStatus::Active`] (drops exited, kicked, or removed accounts);
/// * drain [`PendingValidators`] and append new entrants whose lock is `Active`.
///
/// Returns `Some(_)` only when the resulting set differs from the previous
/// active set, matching the `pallet-session` convention.
impl<T: Config> pallet_session::SessionManager<T::AccountId> for Pallet<T> {
    fn new_session(_new_index: u32) -> Option<alloc::vec::Vec<T::AccountId>> {
        Self::process_equivocations();
        Self::process_offline_counters();

        let previous = DesiredValidators::<T>::get();

        let is_active = |who: &T::AccountId| {
            ValidatorLocks::<T>::get(who)
                .map(|info| info.status == ValidatorStatus::Active)
                .unwrap_or(false)
        };
        
        // A dropped candidate keeps its stake until `expiry_block`, like every
        // other way out of the set. Only the status has to leave `Active`,
        // otherwise auto-renewal would keep extending a lock that no longer
        // belongs to anyone in the set.
        let drop_pending_candidate = |who: &T::AccountId| {
            ValidatorLocks::<T>::mutate(who, |maybe_info| {
                if let Some(info) = maybe_info {
                    info.status = ValidatorStatus::ExitRequested;
                }
            });
            OfflineSessionCount::<T>::remove(who);
            OfflineThisSession::<T>::remove(who);
            Self::deposit_event(Event::PendingValidatorDropped { who: who.clone() });
        };

        let mut next: BoundedVec<T::AccountId, T::MaxValidators> = BoundedVec::default();
        for who in previous.iter() {
            if is_active(who) {
                // Bound is identical to `previous`'s, so this push cannot exceed it.
                let _ = next.try_push(who.clone());
            }
        }

        let pending = PendingValidators::<T>::take();
        for who in pending.into_iter() {
            if !is_active(&who) || next.iter().any(|a| a == &who) {
                continue;
            }
            if !T::SessionInterface::has_keys(&who)
            || next.is_full() {
                drop_pending_candidate(&who);
                continue;
            }
            let _ = next.try_push(who);
        }

        // Always commit the recomputed set to our own storage so that
        // downstream checks (e.g. `lock`'s membership gate) see the truth,
        // even when we hide an empty set from `pallet-session` below.
        DesiredValidators::<T>::put(&next);

        if next == previous {
            return None;
        }
        // Empty-set fallback
        //
        // If every active validator was just dropped (exit, kick, or expiry)
        // and no replacement is pending, returning `Some(empty)` would hand
        // `pallet-session` an empty authority set and brick downstream
        // consumers (`pallet-grandpa`, `pallet-im-online`). Instead we return
        // `None` so the previous authority set is retained on-chain. Those
        // validators have no lock anymore, so they will not actually vote;
        // GRANDPA finality stalls naturally while PoW keeps producing blocks
        // until a new validator locks and the next session boundary swaps
        // the set in. Our own `DesiredValidators` storage was updated above
        // so callers querying membership see the real state.
        if next.is_empty() && !previous.is_empty() {
            return None;
        }
        Some(next.into_inner())
    }

    fn end_session(_end_index: u32) {}

    fn start_session(_start_index: u32) {}

    fn new_session_genesis(_new_index: u32) -> Option<alloc::vec::Vec<T::AccountId>> { Some(DesiredValidators::<T>::get().into_inner()) }
}

/// `pallet-session/historical` specialization. We do not maintain a separate
/// full-identification for validators (no nominator/exposure data), so the
/// identification is `()`. The new-session set is the same one produced by
/// the base `SessionManager`, zipped with unit identifications.
impl<T: Config> pallet_session::historical::SessionManager<T::AccountId, ()> for Pallet<T> {
    fn new_session(new_index: u32) -> Option<alloc::vec::Vec<(T::AccountId, ())>> {
        <Self as pallet_session::SessionManager<T::AccountId>>::new_session(new_index)
            .map(|v| v.into_iter().map(|id| (id, ())).collect())
    }

    fn end_session(end_index: u32) {
        <Self as pallet_session::SessionManager<T::AccountId>>::end_session(end_index);
    }

    fn start_session(start_index: u32) {
        <Self as pallet_session::SessionManager<T::AccountId>>::start_session(start_index);
    }

    fn new_session_genesis(new_index: u32) -> Option<alloc::vec::Vec<(T::AccountId, ())>> {
        <Self as pallet_session::SessionManager<T::AccountId>>::new_session_genesis(new_index)
            .map(|v| v.into_iter().map(|id| (id, ())).collect())
    }

}
