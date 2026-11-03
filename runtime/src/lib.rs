#![cfg_attr(not(feature = "std"), no_std)]
#![recursion_limit = "512"]

#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

pub mod apis;
#[cfg(feature = "runtime-benchmarks")]
mod benchmarks;
pub mod check_nonce;
pub mod configs;
pub mod identity_info;
pub mod inherent_checks;
pub mod migrations;
pub mod weights;

extern crate alloc;
use alloc::vec::Vec;
use sp_runtime::{
	generic, impl_opaque_keys,
	traits::{BlakeTwo256, IdentifyAccount, Verify},
	MultiAddress, MultiSignature,
};
use sp_version::RuntimeVersion;

pub use frame_system::Call as SystemCall;
pub use pallet_balances::Call as BalancesCall;
pub use pallet_timestamp::Call as TimestampCall;
#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;

pub mod genesis_config_presets;

/// Opaque types. These are used by the CLI to instantiate machinery that don't need to know
/// the specifics of the runtime. They can then be made to be agnostic over specific formats
/// of data like extrinsics, allowing for them to continue syncing the network through upgrades
/// to even the core data structures.
pub mod opaque {
	use super::*;
	use sp_runtime::{
		generic,
		traits::{BlakeTwo256, Hash as HashT},
	};

	pub use sp_runtime::OpaqueExtrinsic as UncheckedExtrinsic;

	/// Opaque block header type.
	pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
	/// Opaque block type.
	pub type Block = generic::Block<Header, UncheckedExtrinsic>;
	/// Opaque block identifier type.
	pub type BlockId = generic::BlockId<Block>;
	/// Opaque block hash type.
	pub type Hash = <BlakeTwo256 as HashT>::Output;
}

impl_opaque_keys! {
	pub struct SessionKeys {
		pub grandpa: Grandpa,
		pub im_online: ImOnline,
	}
}

// To learn more about runtime versioning, see:
// https://docs.substrate.io/main-docs/build/upgrade#runtime-versioning
#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
	spec_name: alloc::borrow::Cow::Borrowed("numen-runtime"),
	impl_name: alloc::borrow::Cow::Borrowed("numen-runtime"),
	authoring_version: 1,
	spec_version: 10100,
	impl_version: 1,
	apis: apis::RUNTIME_API_VERSIONS,
	transaction_version: 1,
	system_version: 1,
};


// Time is measured by number of blocks.
pub const MINUTES: BlockNumber = (60 / configs::TargetBlockTime::get()) as BlockNumber;
pub const HOURS: BlockNumber = MINUTES * 60;
pub const DAYS: BlockNumber = HOURS * 24;

pub const BLOCK_HASH_COUNT: BlockNumber = 2400;

// Unit = the base number of indivisible units for balances
// 18 decimals: 1 UNIT = 10^18 smallest units (consistent with EVM/ETH)
pub const       UNIT: Balance = 1_000_000_000_000_000_000;
pub const MILLI_UNIT: Balance = 1_000_000_000_000_000;
pub const MICRO_UNIT: Balance = 1_000_000_000_000;

/// Existential deposit.
///
/// One plank, the least pallet-revive needs to give a contract account a
/// provider. Reaping resets a nonce and would let signed transactions replay, so
/// [`check_nonce::CheckNonceKeepSigned`] keeps every account that ever signed
/// out of reach of it.
pub const EXISTENTIAL_DEPOSIT: Balance = 1;

/// Alias to 512-bit hash when used in the context of a transaction signature on the chain.
pub type Signature = MultiSignature;

/// Some way of identifying an account on the chain. We intentionally make it equivalent
/// to the public key of our transaction signing scheme.
pub type AccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;

/// Balance of an account.
pub type Balance = u128;

/// Index of a transaction in the chain.
pub type Nonce = u32;

/// A hash of some data used by the chain.
pub type Hash = sp_core::H256;

/// An index to a block.
pub type BlockNumber = u32;

/// The address format for describing accounts.
pub type Address = MultiAddress<AccountId, ()>;

/// Block header type as expected by this runtime.
pub type Header = generic::Header<BlockNumber, BlakeTwo256>;

/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;

/// A Block signed with a Justification
pub type SignedBlock = generic::SignedBlock<Block>;

/// BlockId type as expected by this runtime.
pub type BlockId = generic::BlockId<Block>;

/// The `TransactionExtension` to the basic transaction logic.
pub type TxExtension = (
	frame_system::AuthorizeCall<Runtime>,
	frame_system::CheckNonZeroSender<Runtime>,
	frame_system::CheckSpecVersion<Runtime>,
	frame_system::CheckTxVersion<Runtime>,
	frame_system::CheckGenesis<Runtime>,
	frame_system::CheckEra<Runtime>,
	check_nonce::CheckNonceKeepSigned<Runtime>,
	frame_system::CheckWeight<Runtime>,
	pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
	frame_metadata_hash_extension::CheckMetadataHash<Runtime>,
	frame_system::WeightReclaim<Runtime>,
);

/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic =
	generic::UncheckedExtrinsic<Address, RuntimeCall, Signature, TxExtension>;

/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<RuntimeCall, TxExtension>;

/// All migrations of the runtime, aside from the ones declared in the pallets.
///
/// This can be a tuple of types, each implementing `OnRuntimeUpgrade`.
type Migrations = (migrations::AnchorFeeMultiplier,);

/// Executive: handles dispatch to the various modules.
pub type Executive = frame_executive::Executive<
	Runtime,
	Block,
	frame_system::ChainContext<Runtime>,
	Runtime,
	AllPalletsWithSystem,
	Migrations,
>;

// Create the runtime by composing the FRAME pallets that were previously configured.
#[frame_support::runtime]
mod runtime {
	#[runtime::runtime]
	#[runtime::derive(
		RuntimeCall,
		RuntimeEvent,
		RuntimeError,
		RuntimeOrigin,
		RuntimeFreezeReason,
		RuntimeHoldReason,
		RuntimeSlashReason,
		RuntimeLockId,
		RuntimeTask,
		RuntimeViewFunction
	)]
	pub struct Runtime;

	#[runtime::pallet_index(0)]
	pub type System = frame_system;

	#[runtime::pallet_index(1)]
	pub type Timestamp = pallet_timestamp;

	#[runtime::pallet_index(2)]
	pub type Utility = pallet_utility;

	#[runtime::pallet_index(3)]
	pub type Balances = pallet_balances;

	#[runtime::pallet_index(4)]
	pub type TransactionPayment = pallet_transaction_payment;

	#[runtime::pallet_index(5)]
	pub type Multisig = pallet_multisig;

	#[runtime::pallet_index(6)]
	pub type Proxy = pallet_proxy;

	#[runtime::pallet_index(7)]
	pub type Vesting = pallet_vesting;
	
	#[runtime::pallet_index(8)]
	pub type Identity = pallet_identity;

	// --- POW ---

	#[runtime::pallet_index(9)]
	pub type Poscan = configs::poscan::pallet_poscan;

	#[runtime::pallet_index(10)]
	pub type BlockReward = pallet_reward;

	#[runtime::pallet_index(11)]
	pub type Difficulty = pallet_difficulty;

	// --- BFT ---

	#[runtime::pallet_index(12)]
	pub type Grandpa = pallet_grandpa;

	#[runtime::pallet_index(13)]
	pub type Validator = pallet_validator;

	#[runtime::pallet_index(14)]
	pub type ImOnline = pallet_im_online;

	#[runtime::pallet_index(15)]
	pub type Session = pallet_session;

	#[runtime::pallet_index(16)]
	pub type Authorship = pallet_authorship;

	#[runtime::pallet_index(17)]
	pub type Historical = pallet_session::historical;

	// --- OpenGov ---

	#[runtime::pallet_index(18)]
	pub type Preimage = pallet_preimage;

	#[runtime::pallet_index(19)]
	pub type Scheduler = pallet_scheduler;

	#[runtime::pallet_index(20)]
	pub type ConvictionVoting = pallet_conviction_voting;

	#[runtime::pallet_index(21)]
	pub type Referenda = pallet_referenda;

	#[runtime::pallet_index(22)]
	pub type Origins = configs::governance::pallet_custom_origins;

	// --- Treasury and bounties ---

	#[runtime::pallet_index(23)]
	pub type Treasury = pallet_treasury;

	#[runtime::pallet_index(24)]
	pub type Bounties = pallet_bounties;

	#[runtime::pallet_index(25)]
	pub type ChildBounties = pallet_child_bounties;

	// --- EVM ---

	#[runtime::pallet_index(30)]
	pub type Precompiles = configs::evm::pallet_precompiles;

	// ---

	#[runtime::pallet_index(31)]
	pub type GenesisRegistrars = configs::identity::pallet_genesis_registrars;

	#[runtime::pallet_index(255)]
	pub type Prime = pallet_prime;
}

// Offchain workers submit pallet-im-online heartbeats as authorized
// transactions and pallet-grandpa equivocation reports as bare extrinsics.
impl<LocalCall> frame_system::offchain::CreateTransactionBase<LocalCall> for Runtime where RuntimeCall: From<LocalCall>,
{
	type Extrinsic = UncheckedExtrinsic;
	type RuntimeCall = RuntimeCall;
}

impl<LocalCall> frame_system::offchain::CreateBare<LocalCall> for Runtime where RuntimeCall: From<LocalCall>,
{
	fn create_bare(call: RuntimeCall) -> UncheckedExtrinsic {
		UncheckedExtrinsic::new_bare(call)
	}
}

impl<LocalCall> frame_system::offchain::CreateTransaction<LocalCall> for Runtime where RuntimeCall: From<LocalCall>,
{
	type Extension = TxExtension;

	fn create_transaction(call: RuntimeCall, extension: TxExtension) -> UncheckedExtrinsic {
		UncheckedExtrinsic::new_transaction(call, extension)
	}
}

impl<LocalCall> frame_system::offchain::CreateAuthorizedTransaction<LocalCall> for Runtime where RuntimeCall: From<LocalCall>,
{
	fn create_extension() -> TxExtension {
		(
			frame_system::AuthorizeCall::<Runtime>::new(),
			frame_system::CheckNonZeroSender::<Runtime>::new(),
			frame_system::CheckSpecVersion::<Runtime>::new(),
			frame_system::CheckTxVersion::<Runtime>::new(),
			frame_system::CheckGenesis::<Runtime>::new(),
			frame_system::CheckEra::<Runtime>::from(generic::Era::Immortal),
			check_nonce::CheckNonceKeepSigned::<Runtime>::from(0),
			frame_system::CheckWeight::<Runtime>::new(),
			pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
			frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
			frame_system::WeightReclaim::<Runtime>::new(),
		)
	}
}
