// Substrate and Polkadot dependencies
use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use frame_support::{
	derive_impl, parameter_types,
	traits::{
		fungible::{Balanced, Credit, HoldConsideration},
		tokens::{PayFromAccount, UnityAssetBalanceConversion},
		ConstU128, ConstU32, ConstU64, ConstU8, EqualPrivilegeOnly, InstanceFilter,
		LinearStoragePrice, OnUnbalanced, VariantCountOf, WithdrawReasons,
	},
	weights::{
		constants::{RocksDbWeight, WEIGHT_REF_TIME_PER_SECOND},
		ConstantMultiplier, Weight,
	},
	PalletId,
};
use frame_system::{
	limits::{BlockLength, BlockWeights},
	EnsureRoot,
};
use pallet_session::PeriodicSessions;
use pallet_transaction_payment::{FungibleAdapter, Multiplier, TargetedFeeAdjustment};
use scale_info::TypeInfo;
use sp_runtime::{
	traits::{AccountIdConversion, BlakeTwo256, ConvertInto, IdentityLookup, Verify},
	FixedPointNumber, Perbill, Permill, Perquintill,
};
use sp_version::RuntimeVersion;

pub mod evm;
pub mod governance;
pub mod poscan;

// Local module imports
use super::{
	AccountId, Balance, Balances, Block, BlockNumber, Bounties, ChildBounties, Hash, Nonce,
	OriginCaller, PalletInfo, Preimage, Runtime, RuntimeCall, RuntimeEvent, RuntimeFreezeReason,
	RuntimeHoldReason, RuntimeOrigin, RuntimeTask, Session, SessionKeys, Signature, System,
	Treasury, Validator, EXISTENTIAL_DEPOSIT, UNIT, VERSION, DAYS, MINUTES
};

pub(crate) const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(75);

parameter_types! {
	pub const BlockHashCount: BlockNumber = 2400;
	pub const Version: RuntimeVersion = VERSION;

	pub const TargetBlockTime: u64 = 10;

	pub RuntimeBlockWeights: BlockWeights = BlockWeights::with_sensible_defaults(
		Weight::from_parts(2u64 * WEIGHT_REF_TIME_PER_SECOND, u64::MAX),
		NORMAL_DISPATCH_RATIO,
	);
	pub RuntimeBlockLength: BlockLength = BlockLength::builder()
		.max_length(5 * 1024 * 1024)
		.modify_max_length_for_class(
			frame_support::dispatch::DispatchClass::Normal,
			|max| *max = NORMAL_DISPATCH_RATIO * (5 * 1024 * 1024),
		)
		.build();
	pub const SS58Prefix: u16 = 14240;
}

#[cfg(not(feature = "zombienet-runtime"))]
parameter_types! {
	pub const DifficultyHalflife: u64 = 1800;
	pub const DifficultyBreakThresholdSecs: u64 = 1800;
}

#[cfg(feature = "zombienet-runtime")]
parameter_types! {
	pub const DifficultyHalflife: u64 = 60;
	pub const DifficultyBreakThresholdSecs: u64 = 120;
}

/// The default types are being injected by [`derive_impl`](`frame_support::derive_impl`) from
/// [`SoloChainDefaultConfig`](`struct@frame_system::config_preludes::SolochainDefaultConfig`),
/// but overridden as needed.
#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
	/// The block type for the runtime.
	type Block = Block;
	/// Block & extrinsics weights: base values and limits.
	type BlockWeights = RuntimeBlockWeights;
	/// The maximum length of a block (in bytes).
	type BlockLength = RuntimeBlockLength;
	/// The identifier used to distinguish between accounts.
	type AccountId = AccountId;
	/// The type for storing how many extrinsics an account has signed.
	type Nonce = Nonce;
	/// The type for hashing blocks and tries.
	type Hash = Hash;
	/// Maximum number of block number to block hash mappings to keep (oldest pruned first).
	type BlockHashCount = BlockHashCount;
	/// The weight of database operations that the runtime can invoke.
	type DbWeight = RocksDbWeight;
	/// Version of the runtime.
	type Version = Version;
	/// The data to be stored in an account.
	type AccountData = pallet_balances::AccountData<Balance>;
	/// Prefix 14240 pins base58 addresses to a leading "nu", echoing the
	/// Numen brand.
	type SS58Prefix = SS58Prefix;
	type MaxConsumers = frame_support::traits::ConstU32<16>;
	type SystemWeightInfo = crate::weights::frame_system::WeightInfo<Runtime>;
	type ExtensionsWeightInfo = crate::weights::frame_system_extensions::WeightInfo<Runtime>;
}

impl pallet_timestamp::Config for Runtime {
	type Moment = u64;
	type OnTimestampSet = ();
	type MinimumPeriod = ConstU64<2_000>;
	type WeightInfo = crate::weights::pallet_timestamp::WeightInfo<Runtime>;
}

impl pallet_balances::Config for Runtime {
	type MaxLocks = ConstU32<50>;
	type MaxReserves = ();
	type ReserveIdentifier = [u8; 8];
	/// The type for recording an account's balance.
	type Balance = Balance;
	/// The ubiquitous event type.
	type RuntimeEvent = RuntimeEvent;
	type DustRemoval = ();
	type ExistentialDeposit = ConstU128<EXISTENTIAL_DEPOSIT>;
	type AccountStore = System;
	type WeightInfo = crate::weights::pallet_balances::WeightInfo<Runtime>;
	type FreezeIdentifier = RuntimeFreezeReason;
	type MaxFreezes = VariantCountOf<RuntimeFreezeReason>;
	type RuntimeHoldReason = RuntimeHoldReason;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	type DoneSlashHandler = ();
}

parameter_types! {
	/// Reward for the first halving period, in smallest units.
	pub const InitialReward: Balance = 16 * super::UNIT;
	/// Blocks between reward halvings (~4 years at a 10s block time). The
	/// geometric series caps total mined issuance at `2 * InitialReward *
	/// HalvingInterval` = 400,000,000 UNIT.
	pub const HalvingInterval: BlockNumber = 12_500_000;
}

impl pallet_reward::Config for Runtime {
	type Currency = Balances;
	type FindAuthor = PowFindAuthor;
	type InitialReward = InitialReward;
	type HalvingInterval = HalvingInterval;
	type WeightInfo = pallet_reward::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	pub const TargetBlockFullness: Perquintill = Perquintill::from_percent(25);
	/// Upstream's 7.5e-5 assumes 6 second blocks, so the step scales with the
	/// 10 second block time to hold the same wall clock response. Sustained
	/// full blocks then reach `MaximumMultiplier` in under three days.
	pub AdjustmentVariable: Multiplier = Multiplier::saturating_from_rational(125, 1_000_000);
	/// Floor kept symmetric with `MaximumMultiplier`. The stock 1e-6 lets an
	/// idle chain erode the fee anchor by six orders of magnitude, far looser
	/// than the 2x floor `pallet-base-fee` holds on the EVM side.
	pub MinimumMultiplier: Multiplier = Multiplier::saturating_from_rational(1, 10u128);
	pub MaximumMultiplier: Multiplier = Multiplier::saturating_from_integer(10);
}

pub type SlowAdjustingFeeUpdate<R> = TargetedFeeAdjustment<
	R,
	TargetBlockFullness,
	AdjustmentVariable,
	MinimumMultiplier,
	MaximumMultiplier,
>;

/// Routes transaction fees and tips to the block's PoW miner.
///
/// The miner is resolved through [`PowFindAuthor`] via `pallet-authorship`.
/// Blocks without a PoW author digest (never produced by the canonical chain)
/// drop the credit, burning it.
pub struct DealWithFees;

impl OnUnbalanced<Credit<AccountId, Balances>> for DealWithFees {
	fn on_nonzero_unbalanced(amount: Credit<AccountId, Balances>) {
		if let Some(author) = crate::Authorship::author() {
			let _ = <Balances as Balanced<AccountId>>::resolve(&author, amount);
		}
	}
}

/// Price of one weight unit in smallest units, pinned to what the EVM path
/// charges for the same compute. `WeightPerGas` is 20,000 and the base fee is
/// 1 gwei, so a gas unit costs 1e9 and a weight unit is worth 1e9 / 20,000.
/// Any other value makes one path a cheap bypass around the other.
pub const WEIGHT_FEE: Balance = 50_000;

/// Price of one encoded byte, mirroring Ethereum's 16 gas per non-zero calldata
/// byte at the same 1 gwei base fee. Keeps a length-full block within the same
/// order as a weight-full one so neither dimension is the cheap one to abuse.
pub const LENGTH_FEE: Balance = 16_000_000_000;

impl pallet_transaction_payment::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type OnChargeTransaction = FungibleAdapter<Balances, DealWithFees>;
	type OperationalFeeMultiplier = ConstU8<5>;
	type WeightToFee = ConstantMultiplier<Balance, ConstU128<WEIGHT_FEE>>;
	type LengthToFee = ConstantMultiplier<Balance, ConstU128<LENGTH_FEE>>;
	type FeeMultiplierUpdate = SlowAdjustingFeeUpdate<Runtime>;
	type WeightInfo = crate::weights::pallet_transaction_payment::WeightInfo<Runtime>;
}

impl pallet_prime::Config for Runtime {
	type WeightInfo = pallet_prime::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	/// Deposits track the storage footprint of a pending multisig. Each
	/// signatory adds 32 bytes, priced at the same 0.01 NUMN per byte as
	/// bounty and preimage data.
	pub const MultisigDepositBase: Balance = 5 * UNIT;
	pub const MultisigDepositFactor: Balance = 32 * UNIT / 100;
}

impl pallet_multisig::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = Balances;
	type DepositBase = MultisigDepositBase;
	type DepositFactor = MultisigDepositFactor;
	type MaxSignatories = ConstU32<100>;
	type WeightInfo = crate::weights::pallet_multisig::WeightInfo<Runtime>;
	type BlockNumberProvider = System;
}

impl pallet_utility::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type PalletsOrigin = OriginCaller;
	type WeightInfo = crate::weights::pallet_utility::WeightInfo<Runtime>;
}

parameter_types! {
	/// Deposits track the storage footprint of proxy state, priced at the
	/// same 0.01 NUMN per byte as multisig and bounty data. A proxy entry
	/// holds 37 bytes and an announcement holds 68 bytes.
	pub const ProxyDepositBase: Balance = 5 * UNIT;
	pub const ProxyDepositFactor: Balance = 37 * UNIT / 100;
	pub const AnnouncementDepositBase: Balance = 5 * UNIT;
	pub const AnnouncementDepositFactor: Balance = 68 * UNIT / 100;
}

/// Call classes a proxy delegation may be restricted to.
#[derive(
	Copy,
	Clone,
	Eq,
	PartialEq,
	Ord,
	PartialOrd,
	Encode,
	Decode,
	DecodeWithMemTracking,
	Debug,
	MaxEncodedLen,
	TypeInfo,
	Default,
)]
pub enum ProxyType {
	#[default]
	Any,
	NonTransfer,
	Governance,
}

impl InstanceFilter<RuntimeCall> for ProxyType {
	fn filter(&self, c: &RuntimeCall) -> bool {
		match self {
			ProxyType::Any => true,
			// EVM entry points move native balance as well, so they are
			// fenced off together with direct transfers. Nested calls in a
			// utility batch inherit this filter through the origin.
			// `vested_transfer` is the only vesting call that moves funds, the
			// rest only rewrite locks on an account that already holds them.
			ProxyType::NonTransfer => !matches!(
				c,
				RuntimeCall::Balances(..)
					| RuntimeCall::EVM(..)
					| RuntimeCall::Ethereum(..)
					| RuntimeCall::Vesting(pallet_vesting::Call::vested_transfer { .. })
			),
			ProxyType::Governance => matches!(
				c,
				RuntimeCall::Treasury(..)
					| RuntimeCall::Bounties(..)
					| RuntimeCall::ChildBounties(..)
					| RuntimeCall::ConvictionVoting(..)
					| RuntimeCall::Referenda(..)
					| RuntimeCall::Utility(..)
			),
		}
	}

	fn is_superset(&self, o: &Self) -> bool {
		match (self, o) {
			(x, y) if x == y => true,
			(ProxyType::Any, _) => true,
			(_, ProxyType::Any) => false,
			(ProxyType::NonTransfer, _) => true,
			_ => false,
		}
	}
}

impl pallet_proxy::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = Balances;
	type ProxyType = ProxyType;
	type ProxyDepositBase = ProxyDepositBase;
	type ProxyDepositFactor = ProxyDepositFactor;
	type MaxProxies = ConstU32<32>;
	type WeightInfo = crate::weights::pallet_proxy::WeightInfo<Runtime>;
	type MaxPending = ConstU32<32>;
	type CallHasher = BlakeTwo256;
	type AnnouncementDepositBase = AnnouncementDepositBase;
	type AnnouncementDepositFactor = AnnouncementDepositFactor;
	type BlockNumberProvider = System;
}

parameter_types! {
	/// A grant small enough to fall under this is not worth the storage a
	/// schedule costs. One NUMN is a million times the existential deposit.
	pub const MinVestedTransfer: Balance = UNIT;
	/// `pallet-balances` folds every lock into one frozen figure and drops the
	/// reasons each carries, so this set buys no exemption whatever it holds.
	/// Empty spells out what actually happens, which is that unvested funds are
	/// fenced off from every use. A grant needs free balance beside it or the
	/// account cannot even afford to vest.
	pub UnvestedFundsAllowedWithdrawReasons: WithdrawReasons = WithdrawReasons::empty();
}

impl pallet_vesting::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type BlockNumberToBalance = ConvertInto;
	type MinVestedTransfer = MinVestedTransfer;
	type UnvestedFundsAllowedWithdrawReasons = UnvestedFundsAllowedWithdrawReasons;
	type BlockNumberProvider = System;
	type WeightInfo = crate::weights::pallet_vesting::WeightInfo<Runtime>;
	const MAX_VESTING_SCHEDULES: u32 = 28;
}

impl pallet_difficulty::Config for Runtime {
	type TargetBlockTime = TargetBlockTime;
	type Halflife = DifficultyHalflife;
	type BreakThresholdSecs = DifficultyBreakThresholdSecs;
	type WeightInfo = pallet_difficulty::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	/// Maximum number of blocks a GRANDPA equivocation report transaction is
	/// considered valid for inclusion before expiring from the pool.
	pub const GrandpaReportLongevity: u64 = 25;
}

impl pallet_grandpa::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type MaxAuthorities = ConstU32<1000>;
	type MaxNominators = ConstU32<0>;
	type MaxSetIdSessionEntries = ConstU64<168>;
	type KeyOwnerProof = sp_session::MembershipProof;
	type EquivocationReportSystem = pallet_grandpa::EquivocationReportSystem<
		Self,
		GrandpaOffenceReporter,
		pallet_session::historical::Pallet<Runtime>,
		GrandpaReportLongevity,
	>;
}

/// Block-author finder shared by `pallet-authorship` and `pallet-reward`.
///
/// The internal and external miners write the block author as the payload of a
/// `PreRuntime(POW_ENGINE_ID, _)` digest. Decoding it here lets fee routing
/// ([`DealWithFees`]) credit the miner, enables block reward payouts, and lets
/// the GRANDPA equivocation report pipeline attribute a reporter.
pub struct PowFindAuthor;

impl frame_support::traits::FindAuthor<AccountId> for PowFindAuthor {
	fn find_author<'a, I>(digests: I) -> Option<AccountId>
	where
		I: 'a + IntoIterator<Item = (sp_runtime::ConsensusEngineId, &'a [u8])>,
	{
		use codec::Decode;
		use sp_consensus_pow::POW_ENGINE_ID;

		for (engine, mut data) in digests {
			if engine == POW_ENGINE_ID {
				return AccountId::decode(&mut data).ok();
			}
		}
		None
	}
}

impl pallet_authorship::Config for Runtime {
	type FindAuthor = PowFindAuthor;
	type EventHandler = ();
}

/// `pallet-session/historical` configuration.
///
/// Full identification is unit because this chain does not maintain exposures
/// or nominator stakes. The historical trie is required to validate GRANDPA
/// equivocation key-ownership proofs against the session in which the offence
/// occurred.
impl pallet_session::historical::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type FullIdentification = ();
	type FullIdentificationOf = UnitIdentification;
}

impl pallet_session::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ValidatorId = AccountId;
	type ValidatorIdOf = ConvertInto;
	type ShouldEndSession = PeriodicSessions<SessionPeriod, SessionOffset>;
	type NextSessionRotation = PeriodicSessions<SessionPeriod, SessionOffset>;
	type SessionManager = pallet_session::historical::NoteHistoricalRoot<Runtime, Validator>;
	type SessionHandler = <SessionKeys as sp_runtime::traits::OpaqueKeys>::KeyTypeIdProviders;
	type Keys = SessionKeys;
	type DisablingStrategy = ();
	type WeightInfo = pallet_session::weights::SubstrateWeight<Runtime>;
	type Currency = Balances;
	type KeyDeposit = ConstU128<0>;
}

parameter_types! {
	pub const ValidatorLockId: frame_support::traits::LockIdentifier = *b"validatr";
}

#[cfg(not(feature = "zombienet-runtime"))]
parameter_types! {
	pub const SessionPeriod: BlockNumber = 10 * MINUTES;
	pub const SessionOffset: BlockNumber = 0;
}

#[cfg(not(feature = "zombienet-runtime"))]
impl pallet_validator::Config for Runtime {
	type Currency = Balances;
	type SessionInterface = ValidatorSessionAdapter;
	type SessionPeriod = SessionPeriod;
	type SessionOffset = SessionOffset;
	type LockAmount = ConstU128<{ 100_000_000 * UNIT }>;
	type ExemptOrigin = pallet_prime::EnsurePrime<Runtime>;
	type IdentityGate = governance::QualifiedIdentity;
	type LockDuration = ConstU32<{ 180 * DAYS }>;
	type LockId = ValidatorLockId;
	type MaxValidators = ConstU32<1_000>;
	#[allow(clippy::identity_op)]
	type RenewInterval = ConstU32<{ 1 * DAYS }>;
	type OfflineThreshold = ConstU32<1>;
	#[allow(clippy::identity_op)]
	type RejoinCooldownPeriod = ConstU32<{ 1 * DAYS }>;
	type WeightInfo = pallet_validator::weights::SubstrateWeight<Runtime>;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = ValidatorBenchmarkHelper;
}

#[cfg(feature = "zombienet-runtime")]
parameter_types! {
	pub const SessionPeriod: BlockNumber = 5 * MINUTES;
	pub const SessionOffset: BlockNumber = 0 * MINUTES;
}

#[cfg(feature = "zombienet-runtime")]
impl pallet_validator::Config for Runtime {
	type Currency = Balances;
	type SessionInterface = ValidatorSessionAdapter;
	type SessionPeriod = SessionPeriod;
	type SessionOffset = SessionOffset;
	type LockAmount = ConstU128<{ 1 * UNIT }>;
	type ExemptOrigin = pallet_prime::EnsurePrime<Runtime>;
	type IdentityGate = governance::QualifiedIdentity;
	type LockDuration = ConstU32<{ SessionPeriod::get() + 2 * MINUTES }>;
	type LockId = ValidatorLockId;
	type MaxValidators = ConstU32<4>;
	type RenewInterval = ConstU32<{ 3 * MINUTES }>;
	type OfflineThreshold = ConstU32<1>;
	type RejoinCooldownPeriod = ConstU32<{ SessionPeriod::get() + 5 * MINUTES }>;
	type WeightInfo = pallet_validator::weights::SubstrateWeight<Runtime>;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = ValidatorBenchmarkHelper;
}

/// Adapter wiring `pallet_validator::SessionInterface` to `pallet-session`.
/// Lets validator verify session keys exist before queuing a candidate.
pub struct ValidatorSessionAdapter;

impl pallet_validator::SessionInterface<AccountId> for ValidatorSessionAdapter {
	fn has_keys(who: &AccountId) -> bool {
		pallet_session::NextKeys::<Runtime>::contains_key(who)
	}
}

/// Walks a benchmark account past both validator entry gates. The keys are
/// zero bytes since `has_keys` only tests for presence and nothing signs with
/// them during a benchmark.
#[cfg(feature = "runtime-benchmarks")]
pub struct ValidatorBenchmarkHelper;

#[cfg(feature = "runtime-benchmarks")]
impl pallet_validator::BenchmarkHelper<AccountId> for ValidatorBenchmarkHelper {
	fn make_eligible(who: &AccountId) {
		governance::qualify_identity(who);
		let keys = SessionKeys::decode(&mut sp_runtime::traits::TrailingZeroInput::zeroes())
			.expect("session keys decode from zero bytes");
		pallet_session::NextKeys::<Runtime>::insert(who, keys);
	}
}
/// `ValidatorSetWithIdentification` adapter over `pallet-session`.
///
/// `pallet-im-online` requires its `ValidatorSet` to also expose an
/// `Identification` type so that offline reports can carry a per-validator
/// payload. We do not run a separate slashing/staking subsystem yet, so the
/// identification is a unit value.
pub struct UnitIdentification;

impl sp_runtime::traits::Convert<AccountId, Option<()>> for UnitIdentification {
	fn convert(_: AccountId) -> Option<()> {
		Some(())
	}
}

pub struct ValidatorIdentification;

impl frame_support::traits::ValidatorSet<AccountId> for ValidatorIdentification {
	type ValidatorId = <Session as frame_support::traits::ValidatorSet<AccountId>>::ValidatorId;
	type ValidatorIdOf = <Session as frame_support::traits::ValidatorSet<AccountId>>::ValidatorIdOf;

	fn session_index() -> sp_staking::SessionIndex {
		<Session as frame_support::traits::ValidatorSet<AccountId>>::session_index()
	}

	fn validators() -> alloc::vec::Vec<Self::ValidatorId> {
		<Session as frame_support::traits::ValidatorSet<AccountId>>::validators()
	}
}

impl frame_support::traits::ValidatorSetWithIdentification<AccountId> for ValidatorIdentification {
	type Identification = ();
	type IdentificationOf = UnitIdentification;
}


parameter_types! {
	/// Base priority for unsigned heartbeat extrinsics. Picked to be low enough
	/// not to crowd out other unsigned traffic but high enough to land within
	/// the session.
	pub const ImOnlineUnsignedPriority: sp_runtime::transaction_validity::TransactionPriority =
		sp_runtime::transaction_validity::TransactionPriority::MAX / 2;
	/// Maximum number of `ImOnlineId` keys stored per session.
	pub const ImOnlineMaxKeys: u32 = 1_000;
	/// Maximum peers reported in a single heartbeat payload.
	pub const ImOnlineMaxPeerInHeartbeats: u32 = 10_000;
}

pub struct ImOnlineOffenceReporter;

impl<O>
	sp_staking::offence::ReportOffence<AccountId, (AccountId, ()), O>
	for ImOnlineOffenceReporter
where
	O: sp_staking::offence::Offence<(AccountId, ())>,
{
	fn report_offence(
		_reporters: alloc::vec::Vec<AccountId>,
		offence: O,
	) -> Result<(), sp_staking::offence::OffenceError> {
		for (offender, _) in offence.offenders() {
			pallet_validator::Pallet::<Runtime>::note_offline(&offender);
		}
		Ok(())
	}

	fn is_known_offence(_offenders: &[(AccountId, ())], _time_slot: &O::TimeSlot) -> bool {
		false
	}
}

/// Offence reporter that forwards GRANDPA equivocation reports into
/// `pallet-validator`.
///
/// The `pallet-grandpa` built-in [`pallet_grandpa::EquivocationReportSystem`]
/// expects a [`ReportOffence`] implementation keyed by the
/// `IdentificationTuple` produced by `pallet-session/historical`, which in
/// this runtime is `(AccountId, ())`. Upon receiving a verified offence we
/// call [`pallet_validator::Pallet::note_equivocation`] for the offender,
/// which switches the validator's lock to `Kicked`, records the rejoin
/// cooldown deadline, and emits the `ValidatorKicked { Equivocation }`
/// event. The next session boundary removes the validator from the active
/// set via the existing session manager logic.
pub struct GrandpaOffenceReporter;

impl<O>
	sp_staking::offence::ReportOffence<AccountId, (AccountId, ()), O>
	for GrandpaOffenceReporter
where
	O: sp_staking::offence::Offence<(AccountId, ())>,
{
	fn report_offence(
		_reporters: alloc::vec::Vec<AccountId>,
		offence: O,
	) -> Result<(), sp_staking::offence::OffenceError> {
		for (offender, _) in offence.offenders() {
			pallet_validator::Pallet::<Runtime>::note_equivocation(&offender);
		}
		Ok(())
	}

	fn is_known_offence(_offenders: &[(AccountId, ())], _time_slot: &O::TimeSlot) -> bool {
		false
	}
}

impl pallet_im_online::Config for Runtime {
	type AuthorityId = pallet_im_online::sr25519::AuthorityId;
	type RuntimeEvent = RuntimeEvent;
	type ValidatorSet = ValidatorIdentification;
	type NextSessionRotation = PeriodicSessions<SessionPeriod, SessionOffset>;
	type ReportUnresponsiveness = ImOnlineOffenceReporter;
	type UnsignedPriority = ImOnlineUnsignedPriority;
	type MaxKeys = ImOnlineMaxKeys;
	type MaxPeerInHeartbeats = ImOnlineMaxPeerInHeartbeats;
	type WeightInfo = crate::weights::pallet_im_online::WeightInfo<Runtime>;
}

parameter_types! {
	pub const TreasuryPalletId: PalletId = PalletId(*b"py/trsry");
	pub const SpendPeriod: BlockNumber = DAYS;
	/// Treasury holds the genesis endowment with no ongoing income, so idle
	/// funds must not decay.
	pub const Burn: Permill = Permill::zero();
	pub const PayoutPeriod: BlockNumber = 30 * DAYS;
	pub const MaxApprovals: u32 = 100;
	pub TreasuryAccount: AccountId = TreasuryPalletId::get().into_account_truncating();
}

impl pallet_treasury::Config for Runtime {
	type PalletId = TreasuryPalletId;
	type Currency = Balances;
	type RejectOrigin = pallet_prime::EnsurePrime<Runtime>;
	type RuntimeEvent = RuntimeEvent;
	type SpendPeriod = SpendPeriod;
	type Burn = Burn;
	type BurnDestination = ();
	type SpendFunds = Bounties;
	type WeightInfo = pallet_treasury::weights::SubstrateWeight<Runtime>;
	type MaxApprovals = MaxApprovals;
	type SpendOrigin = governance::TreasurySpender;
	type AssetKind = ();
	type Beneficiary = AccountId;
	type BeneficiaryLookup = IdentityLookup<AccountId>;
	type Paymaster = PayFromAccount<Balances, TreasuryAccount>;
	type BalanceConverter = UnityAssetBalanceConversion;
	type PayoutPeriod = PayoutPeriod;
	type BlockNumberProvider = System;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = ();
}

parameter_types! {
	pub const BountyDepositBase: Balance = 10 * UNIT;
	pub const BountyDepositPayoutDelay: BlockNumber = DAYS;
	pub const BountyUpdatePeriod: BlockNumber = 90 * DAYS;
	pub const CuratorDepositMultiplier: Permill = Permill::from_percent(50);
	pub const CuratorDepositMin: Option<Balance> = Some(10 * UNIT);
	pub const CuratorDepositMax: Option<Balance> = Some(1_000 * UNIT);
	pub const BountyValueMinimum: Balance = UNIT;
	pub const DataDepositPerByte: Balance = UNIT / 100;
	pub const MaximumReasonLength: u32 = 300;
}

impl pallet_bounties::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type BountyDepositBase = BountyDepositBase;
	type BountyDepositPayoutDelay = BountyDepositPayoutDelay;
	type BountyUpdatePeriod = BountyUpdatePeriod;
	type CuratorDepositMultiplier = CuratorDepositMultiplier;
	type CuratorDepositMin = CuratorDepositMin;
	type CuratorDepositMax = CuratorDepositMax;
	type BountyValueMinimum = BountyValueMinimum;
	type DataDepositPerByte = DataDepositPerByte;
	type MaximumReasonLength = MaximumReasonLength;
	type WeightInfo = crate::weights::pallet_bounties::WeightInfo<Runtime>;
	type ChildBountyManager = ChildBounties;
	type OnSlash = Treasury;
	type TransferAllAssets = ();
}

parameter_types! {
	pub const ChildBountyValueMinimum: Balance = UNIT;
	pub const MaxActiveChildBountyCount: u32 = 1000;
}

impl pallet_child_bounties::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type MaxActiveChildBountyCount = MaxActiveChildBountyCount;
	type ChildBountyValueMinimum = ChildBountyValueMinimum;
	type WeightInfo = pallet_child_bounties::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	pub MaximumSchedulerWeight: Weight =
		Perbill::from_percent(80) * RuntimeBlockWeights::get().max_block;
}

impl pallet_scheduler::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeOrigin = RuntimeOrigin;
	type PalletsOrigin = OriginCaller;
	type RuntimeCall = RuntimeCall;
	type MaximumWeight = MaximumSchedulerWeight;
	type ScheduleOrigin = EnsureRoot<AccountId>;
	#[cfg(feature = "runtime-benchmarks")]
	type MaxScheduledPerBlock = ConstU32<512>;
	#[cfg(not(feature = "runtime-benchmarks"))]
	type MaxScheduledPerBlock = ConstU32<50>;
	type WeightInfo = crate::weights::pallet_scheduler::WeightInfo<Runtime>;
	type OriginPrivilegeCmp = EqualPrivilegeOnly;
	type Preimages = Preimage;
	type BlockNumberProvider = System;
}

parameter_types! {
	pub const PreimageBaseDeposit: Balance = 5 * UNIT;
	pub const PreimageByteDeposit: Balance = UNIT / 100;
	pub const PreimageHoldReason: RuntimeHoldReason =
		RuntimeHoldReason::Preimage(pallet_preimage::HoldReason::Preimage);
}

impl pallet_preimage::Config for Runtime {
	type WeightInfo = crate::weights::pallet_preimage::WeightInfo<Runtime>;
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type ManagerOrigin = EnsureRoot<AccountId>;
	type Consideration = HoldConsideration<
		AccountId,
		Balances,
		PreimageHoldReason,
		LinearStoragePrice<PreimageBaseDeposit, PreimageByteDeposit, Balance>,
	>;
}

parameter_types! {
	/// Held for the fixed part of a `Registration` outside its `IdentityInfo`,
	/// 17 bytes at the 0.01 NUMN per byte storage price plus the 5 NUMN storage
	/// entry base shared with multisig and proxy.
	pub const IdentityBasicDeposit: Balance = 5 * UNIT + 17 * UNIT / 100;
	/// Priced per encoded byte of the `IdentityInfo` itself, at the same
	/// 0.01 NUMN per byte.
	pub const IdentityByteDeposit: Balance = UNIT / 100;
	/// A username adds 32 bytes of on-chain state.
	pub const UsernameDeposit: Balance = 32 * UNIT / 100;
	/// A sub-account adds 53 bytes across two trie items.
	pub const SubAccountDeposit: Balance = 5 * UNIT + 53 * UNIT / 100;
}

impl pallet_identity::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type BasicDeposit = IdentityBasicDeposit;
	type ByteDeposit = IdentityByteDeposit;
	type UsernameDeposit = UsernameDeposit;
	type SubAccountDeposit = SubAccountDeposit;
	type MaxSubAccounts = ConstU32<100>;
	type IdentityInformation = crate::identity_info::IdentityInfo;
	type MaxRegistrars = ConstU32<1000>;
	type Slashed = Treasury;
	// Unreachable with no Root track and no sudo. Killing an identity only clears
	// current state. The payload stays in the block that set it, so the power
	// removes nothing while handing prime a way to slash someone's deposit.
	type ForceOrigin = EnsureRoot<AccountId>;
	type RegistrarOrigin = pallet_prime::EnsurePrime<Runtime>;
	type OffchainSignature = Signature;
	type SigningPublicKey = <Signature as Verify>::Signer;
	type UsernameAuthorityOrigin = pallet_prime::EnsurePrime<Runtime>;
	type PendingUsernameExpiration = ConstU32<{ 7 * DAYS }>;
	type UsernameGracePeriod = ConstU32<{ 30 * DAYS }>;
	type MaxSuffixLength = ConstU32<7>;
	type MaxUsernameLength = ConstU32<32>;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = ();
	type WeightInfo = pallet_identity::weights::SubstrateWeight<Runtime>;
}
