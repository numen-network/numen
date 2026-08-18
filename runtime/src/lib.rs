#![cfg_attr(not(feature = "std"), no_std)]
#![recursion_limit = "512"]

#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

pub mod apis;
#[cfg(feature = "runtime-benchmarks")]
mod benchmarks;
pub mod configs;
pub mod identity_info;
pub mod inherent_checks;
pub mod weights;

extern crate alloc;
use alloc::vec::Vec;
use sp_runtime::{
	generic, impl_opaque_keys,
	traits::{BlakeTwo256, IdentifyAccount, Verify},
	MultiAddress, MultiSignature,
};
#[cfg(feature = "std")]
use sp_version::NativeVersion;
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
	// The version of the runtime specification. A full node will not attempt to use its native
	//   runtime in substitute for the on-chain Wasm runtime unless all of `spec_name`,
	//   `spec_version`, and `authoring_version` are the same between Wasm and native.
	// This value is set to 100 to notify Polkadot-JS App (https://polkadot.js.org/apps) to use
	//   the compatible custom types.
	spec_version: 100,
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
pub const EXISTENTIAL_DEPOSIT: Balance = MICRO_UNIT;

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
	NativeVersion { runtime_version: VERSION, can_author_with: Default::default() }
}

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
	frame_system::CheckNonZeroSender<Runtime>,
	frame_system::CheckSpecVersion<Runtime>,
	frame_system::CheckTxVersion<Runtime>,
	frame_system::CheckGenesis<Runtime>,
	frame_system::CheckEra<Runtime>,
	frame_system::CheckNonce<Runtime>,
	frame_system::CheckWeight<Runtime>,
	pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
	frame_metadata_hash_extension::CheckMetadataHash<Runtime>,
	frame_system::WeightReclaim<Runtime>,
);

/// Unchecked extrinsic type as expected by this runtime.
///
/// We use `fp_self_contained::UncheckedExtrinsic` to support Frontier's
/// self-contained Ethereum transactions alongside regular signed Substrate
/// extrinsics. The [`SelfContainedCall`] implementation below delegates to
/// `pallet-ethereum`, so `pallet_ethereum::transact` is dispatched as a
/// self-contained call (signed by an EVM key, bypassing the substrate
/// signed-extension pipeline) while all other calls behave exactly as with
/// the stock `generic::UncheckedExtrinsic`.
pub type UncheckedExtrinsic =
	fp_self_contained::UncheckedExtrinsic<Address, RuntimeCall, Signature, TxExtension>;

/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<RuntimeCall, TxExtension>;

/// All migrations of the runtime, aside from the ones declared in the pallets.
///
/// This can be a tuple of types, each implementing `OnRuntimeUpgrade`.
#[allow(unused_parens)]
type Migrations = ();

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
	pub type Balances = pallet_balances;

	#[runtime::pallet_index(3)]
	pub type TransactionPayment = pallet_transaction_payment;

	#[runtime::pallet_index(4)]
	pub type BlockReward = pallet_reward;

	#[runtime::pallet_index(5)]
	pub type Difficulty = pallet_difficulty;

	#[runtime::pallet_index(6)]
	pub type Grandpa = pallet_grandpa;

	#[runtime::pallet_index(7)]
	pub type Validator = pallet_validator;

	#[runtime::pallet_index(8)]
	pub type ImOnline = pallet_im_online;

	#[runtime::pallet_index(9)]
	pub type Session = pallet_session;

	#[runtime::pallet_index(10)]
	pub type Authorship = pallet_authorship;

	#[runtime::pallet_index(11)]
	pub type Historical = pallet_session::historical;

	// --- Frontier EVM stack ---

	#[runtime::pallet_index(12)]
	pub type Ethereum = pallet_ethereum;

	#[runtime::pallet_index(13)]
	pub type EVM = pallet_evm;

	#[runtime::pallet_index(14)]
	pub type EVMChainId = pallet_evm_chain_id;

	#[runtime::pallet_index(15)]
	pub type BaseFee = pallet_base_fee;

	// --- Treasury and bounties ---

	#[runtime::pallet_index(16)]
	pub type Treasury = pallet_treasury;

	#[runtime::pallet_index(17)]
	pub type Bounties = pallet_bounties;

	#[runtime::pallet_index(18)]
	pub type ChildBounties = pallet_child_bounties;

	// --- OpenGov ---

	#[runtime::pallet_index(19)]
	pub type Preimage = pallet_preimage;

	#[runtime::pallet_index(20)]
	pub type Scheduler = pallet_scheduler;

	#[runtime::pallet_index(21)]
	pub type ConvictionVoting = pallet_conviction_voting;

	#[runtime::pallet_index(22)]
	pub type Referenda = pallet_referenda;

	#[runtime::pallet_index(23)]
	pub type Origins = configs::governance::pallet_custom_origins;

	#[runtime::pallet_index(24)]
	pub type Multisig = pallet_multisig;

	#[runtime::pallet_index(25)]
	pub type Utility = pallet_utility;

	#[runtime::pallet_index(26)]
	pub type Proxy = pallet_proxy;

	#[runtime::pallet_index(27)]
	pub type Prime = pallet_prime;

	// ---

	#[runtime::pallet_index(28)]
	pub type Vesting = pallet_vesting;
	
	#[runtime::pallet_index(29)]
	pub type Identity = pallet_identity;
}

// pallet-im-online submits unsigned heartbeat extrinsics from offchain
// workers. The runtime must expose how to build a bare (unsigned/inherent)
// extrinsic for any pallet `Call`.
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

/// Self-contained Ethereum transaction support.
///
/// `pallet-ethereum`'s `transact` extrinsic is signed by an EVM key rather
/// than a Substrate key, so it bypasses the regular signed-extension
/// pipeline. The implementation simply delegates to the call's
/// [`fp_self_contained::SelfContainedCall`] impl exposed by
/// `pallet-ethereum`.
impl fp_self_contained::SelfContainedCall for RuntimeCall {
	type SignedInfo = sp_core::H160;

	fn is_self_contained(&self) -> bool {
		match self {
			RuntimeCall::Ethereum(call) => call.is_self_contained(),
			_ => false,
		}
	}

	fn check_self_contained(
		&self,
	) -> Option<Result<Self::SignedInfo, sp_runtime::transaction_validity::TransactionValidityError>>
	{
		match self {
			RuntimeCall::Ethereum(call) => call.check_self_contained(),
			_ => None,
		}
	}

	fn validate_self_contained(
		&self,
		info: &Self::SignedInfo,
		dispatch_info: &sp_runtime::traits::DispatchInfoOf<RuntimeCall>,
		len: usize,
	) -> Option<sp_runtime::transaction_validity::TransactionValidity> {
		match self {
			RuntimeCall::Ethereum(call) => call.validate_self_contained(info, dispatch_info, len),
			_ => None,
		}
	}

	fn pre_dispatch_self_contained(
		&self,
		info: &Self::SignedInfo,
		dispatch_info: &sp_runtime::traits::DispatchInfoOf<RuntimeCall>,
		len: usize,
	) -> Option<Result<(), sp_runtime::transaction_validity::TransactionValidityError>> {
		match self {
			RuntimeCall::Ethereum(call) => {
				call.pre_dispatch_self_contained(info, dispatch_info, len)
			}
			_ => None,
		}
	}

	fn apply_self_contained(
		self,
		info: Self::SignedInfo,
	) -> Option<
		sp_runtime::DispatchResultWithInfo<
			sp_runtime::traits::PostDispatchInfoOf<RuntimeCall>,
		>,
	> {
		use sp_runtime::traits::Dispatchable;
		match self {
			call @ RuntimeCall::Ethereum(pallet_ethereum::Call::transact { .. }) => Some(
				call.dispatch(RuntimeOrigin::from(
					pallet_ethereum::RawOrigin::EthereumTransaction(info),
				)),
			),
			_ => None,
		}
	}
}
