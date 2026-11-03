//! EVM configuration.

use frame_support::{
	parameter_types,
	traits::{ConstBool, ConstU32, Get},
};
use frame_system::EnsureSigned;
use sp_runtime::{FixedU128, Perbill};

use super::{DealWithFees, PowFindAuthor};
use crate::{
	AccountId, Address, Balance, Balances, EthExtraImpl, Runtime, RuntimeCall, RuntimeEvent,
	RuntimeHoldReason, RuntimeOrigin, Signature, Timestamp, MICRO_UNIT, MILLI_UNIT,
};

/// Chain id of the dev, local and integration chains, and of any chain whose
/// genesis sets none.
pub const DEV_CHAIN_ID: u64 = 320_262;

parameter_types! {
	/// Deposit per byte of code or storage, the 200 gas Ethereum charges per
	/// byte of deployed code at 1000 gwei.
	pub const DepositPerByte: Balance = 200 * MICRO_UNIT;
	/// Deposit per storage slot on top of its 64 bytes. A new slot then costs
	/// about the 20,000 gas Ethereum charges for it at 1000 gwei.
	pub const DepositPerChildTrieItem: Balance = 8 * MILLI_UNIT;
	/// Deposit per contract record, which together with its bytes comes to about
	/// the 32,000 gas of an Ethereum CREATE at 1000 gwei.
	pub const DepositPerItem: Balance = 8 * MILLI_UNIT;
	pub const CodeHashLockupDepositPercent: Perbill = Perbill::from_percent(30);
	pub MaxEthExtrinsicWeight: FixedU128 = FixedU128::from_rational(9, 10);
	/// Splits the fee of an Ethereum transaction into gas and gas price, sized
	/// so a native transfer stays within the 21,000 gas wallets assume for it.
	pub const GasScale: u32 = 351_000_000;
}

/// Chain id EVM signatures commit to, read from [`pallet_evm_chain_id`] so that
/// every network built from this runtime keeps its own.
pub struct EvmChainId;

impl Get<u64> for EvmChainId {
	fn get() -> u64 {
		pallet_evm_chain_id::ChainId::<Runtime>::get()
	}
}

/// EIP-155 chain id of the network.
#[frame_support::pallet]
pub mod pallet_evm_chain_id {
	use frame_support::{pallet_prelude::*, DefaultNoBound};

	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::type_value]
	pub fn DefaultChainId() -> u64 {
		super::DEV_CHAIN_ID
	}

	#[pallet::storage]
	pub type ChainId<T> = StorageValue<_, u64, ValueQuery, DefaultChainId>;

	#[pallet::genesis_config]
	#[derive(DefaultNoBound)]
	pub struct GenesisConfig<T: Config> {
		pub chain_id: u64,
		#[serde(skip)]
		pub _config: core::marker::PhantomData<T>,
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			ChainId::<T>::put(self.chain_id);
		}
	}
}

impl pallet_evm_chain_id::Config for Runtime {}

/// EVM facts the chain publishes for off chain readers.
#[frame_support::pallet]
pub mod pallet_precompiles {
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
			crate::balances_erc20::ADDRESS
		}
	}
}

impl pallet_precompiles::Config for Runtime {}

impl pallet_revive::Config for Runtime {
	type Time = Timestamp;
	type Balance = Balance;
	type Currency = Balances;
	type OnBurn = DealWithFees;
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type RuntimeOrigin = RuntimeOrigin;
	type RuntimeHoldReason = RuntimeHoldReason;
	type WeightInfo = pallet_revive::weights::SubstrateWeight<Self>;
	type Precompiles = (crate::balances_erc20::BalancesErc20,);
	type FindAuthor = PowFindAuthor;
	type DepositPerByte = DepositPerByte;
	type DepositPerItem = DepositPerItem;
	type DepositPerChildTrieItem = DepositPerChildTrieItem;
	type CodeHashLockupDepositPercent = CodeHashLockupDepositPercent;
	type AddressMapper = pallet_revive::AccountId32Mapper<Self>;
	type AllowEVMBytecode = ConstBool<true>;
	type UploadOrigin = EnsureSigned<AccountId>;
	type InstantiateOrigin = EnsureSigned<AccountId>;
	type RuntimeMemory = ConstU32<{ 128 * 1024 * 1024 }>;
	type PVFMemory = ConstU32<{ 512 * 1024 * 1024 }>;
	type ChainId = EvmChainId;
	type NativeToEthRatio = ConstU32<1>;
	type FeeInfo = pallet_revive::evm::fees::Info<Address, Signature, EthExtraImpl>;
	type Deposit = ();
	type MaxEthExtrinsicWeight = MaxEthExtrinsicWeight;
	type DebugEnabled = ConstBool<false>;
	type AutoMap = ConstBool<true>;
	type GasScale = GasScale;
}
