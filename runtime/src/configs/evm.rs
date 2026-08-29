//! Frontier EVM configuration.
//!
//! Wires `pallet-evm`, `pallet-ethereum`, `pallet-base-fee` and
//! `pallet-evm-chain-id` into the runtime. Substrate `AccountId32` accounts
//! are mapped onto EVM `H160` addresses through Frontier's standard
//! `HashedAddressMapping<BlakeTwo256>`. A PoW miner has no `H160` identity, so
//! the EVM `COINBASE` (`FindAuthor<H160>`) is pinned to the zero address; EVM
//! fees are still routed to the substrate-side miner by [`EvmDealWithFees`].

use frame_support::{
	parameter_types,
	traits::{
		fungible::{Balanced, Credit},
		ConstU32, FindAuthor,
	},
	weights::{constants::WEIGHT_REF_TIME_PER_MILLIS, Weight},
};
use pallet_ethereum::PostLogContent;
use pallet_evm::{
	EnsureAddressNever, EnsureAddressRoot, EVMFungibleAdapter, HashedAddressMapping,
	OnChargeEVMTransaction,
};
use pallet_evm_precompile_bn128::{Bn128Add, Bn128Mul, Bn128Pairing};
use pallet_evm_precompile_balances_erc20::{Erc20BalancesPrecompile, Erc20Metadata};
use pallet_evm_precompile_modexp::Modexp;
use pallet_evm_precompile_simple::{ECRecover, Identity, Ripemd160, Sha256};
use precompile_utils::precompile_set::{
	AcceptDelegateCall, AddressU64, CallableByContract, CallableByPrecompile, PrecompileAt,
	PrecompileSetBuilder,
};
use sp_core::{H160, U256};
use sp_runtime::{
	traits::BlakeTwo256,
	ConsensusEngineId, Permill,
};

use super::{DealWithFees, TreasuryAccount, NORMAL_DISPATCH_RATIO};
use crate::{AccountId, Authorship, Balances, Runtime, Timestamp};

/// Target block gas limit (matches the Frontier template default).
const BLOCK_GAS_LIMIT: u64 = 75_000_000;
/// Compute budget per block, in milliseconds, used to derive `WeightPerGas`.
///
/// PoW targets ~10s of wall-clock per block (see [`super::TargetBlockTime`]),
/// but the runtime caps actual on-chain compute to 2s of reference time
/// (see `RuntimeBlockWeights` in [`super`]). The EVM gas/weight conversion
/// must be calibrated against this real compute budget — not the wall-clock
/// block time — otherwise `WeightPerGas` is under-counted and a single
/// large-gas transaction can exhaust the block weight budget long before
/// reaching [`BLOCK_GAS_LIMIT`].
const WEIGHT_MILLIS_PER_BLOCK: u64 = 2_000;
/// Maximum PoV size (only relevant on parachains; kept for parity with the
/// Frontier template formula).
const MAX_POV_SIZE: u64 = 5 * 1024 * 1024;
/// Soft cap on storage growth per block, used to derive
/// `GasLimitStorageGrowthRatio`.
const MAX_STORAGE_GROWTH: u64 = 400 * 1024;
/// EVM slot the native balance ERC20 facade is mounted at.
const BALANCES_ERC20: u64 = 0x802;

parameter_types! {
	pub BlockGasLimit: U256 = U256::from(BLOCK_GAS_LIMIT);
	pub TransactionGasLimit: Option<U256> = Some(fp_evm::MAX_TRANSACTION_GAS_LIMIT);
	pub const GasLimitPovSizeRatio: u64 = BLOCK_GAS_LIMIT.saturating_div(MAX_POV_SIZE);
	pub const GasLimitStorageGrowthRatio: u64 = BLOCK_GAS_LIMIT.saturating_div(MAX_STORAGE_GROWTH);
	pub WeightPerGas: Weight = Weight::from_parts(
		weight_per_gas(BLOCK_GAS_LIMIT, NORMAL_DISPATCH_RATIO, WEIGHT_MILLIS_PER_BLOCK),
		0,
	);
	pub PrecompilesValue: FrontierPrecompiles<Runtime> = FrontierPrecompiles::<Runtime>::new();
}

/// Local copy of [`fp_evm::weight_per_gas`] used here in `const`-friendly form.
fn weight_per_gas(block_gas_limit: u64, txn_ratio: sp_runtime::Perbill, weight_ms: u64) -> u64 {
	let weight_per_block = WEIGHT_REF_TIME_PER_MILLIS.saturating_mul(weight_ms);
	let w = (txn_ratio * weight_per_block).saturating_div(block_gas_limit);
	core::cmp::max(w, 1)
}

/// PoW does not nominate a block author H160; EVM's `FindAuthor` returns the
/// zero address so opcodes like `COINBASE` evaluate deterministically.
pub struct EvmFindAuthorZero;
impl FindAuthor<H160> for EvmFindAuthorZero {
	fn find_author<'a, I>(_digests: I) -> Option<H160>
	where
		I: 'a + IntoIterator<Item = (ConsensusEngineId, &'a [u8])>,
	{
		Some(H160::zero())
	}
}

/// ERC20 metadata for the native token, injected into the
/// `balances-erc20` precompile.
pub struct NativeErc20Metadata;
impl Erc20Metadata for NativeErc20Metadata {
	const NAME: &'static str = "Numen";
	const SYMBOL: &'static str = "NUMN";
	const DECIMALS: u8 = 18;
}

/// Call context policy for the stock Ethereum precompiles. They are pure
/// functions of their input and mainnet lets every caller reach them, including
/// through DELEGATECALL, so all three doors stay open to keep bytecode compiled
/// for Ethereum working unchanged.
pub type EthereumPrecompileChecks = (AcceptDelegateCall, CallableByContract, CallableByPrecompile);

/// Call context policy for the ERC20 facade. It reads the funds owner from
/// `context().caller`, which DELEGATECALL and CALLCODE rebind to the outer
/// caller, so borrowed code could spend a third party balance. Omitting
/// [`AcceptDelegateCall`] makes `PrecompileAt` reject those two opcodes. Plain
/// CALL stays open to contracts and precompiles, where the caller really is the
/// account being debited.
pub type Erc20PrecompileChecks = (CallableByContract, CallableByPrecompile);

/// Precompile set covering the standard Ethereum precompiles 1-8 plus the
/// chain-specific `balances-erc20` precompile at [`BALANCES_ERC20`], which
/// exposes the native balance pallet through an ERC20 interface and adds a
/// `withdraw(bytes32,uint256)` helper for EVM -> Substrate transfers.
///
/// `ECRecover`, `SHA256`, `RIPEMD160` and `Identity` use
/// [`pallet_evm_precompile_simple`]; modexp uses
/// [`pallet_evm_precompile_modexp`]; the bn128 curve precompiles use
/// [`pallet_evm_precompile_bn128`]; the chain-specific ERC20 facade comes
/// from [`pallet_evm_precompile_balances_erc20`].
///
/// Built with [`PrecompileSetBuilder`] rather than a hand written dispatcher so
/// that every member goes through the upstream call context checks. A dispatcher
/// that only matches on the code address silently accepts DELEGATECALL, which
/// forges `msg.sender` for any precompile that trusts it.
pub type FrontierPrecompiles<R> = PrecompileSetBuilder<
	R,
	(
		PrecompileAt<AddressU64<1>, ECRecover, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<2>, Sha256, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<3>, Ripemd160, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<4>, Identity, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<5>, Modexp, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<6>, Bn128Add, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<7>, Bn128Mul, EthereumPrecompileChecks>,
		PrecompileAt<AddressU64<8>, Bn128Pairing, EthereumPrecompileChecks>,
		PrecompileAt<
			AddressU64<{ BALANCES_ERC20 }>,
			Erc20BalancesPrecompile<R, NativeErc20Metadata>,
			Erc20PrecompileChecks,
		>,
	),
>;

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
		fn balances_erc20() -> H160 {
			H160::from_low_u64_be(BALANCES_ERC20)
		}
	}
}

impl pallet_precompiles::Config for Runtime {}

/// EVM fee handler routing both the base fee and the priority tip to the PoW
/// miner. Base fee goes through [`DealWithFees`]; the tip is deposited to the
/// same author here, overriding the default that pays the `COINBASE` address
/// (which PoW pins to zero, see [`EvmFindAuthorZero`]). Without an author
/// digest the tip falls back to the treasury, like the base fee does.
pub struct EvmDealWithFees;

impl OnChargeEVMTransaction<Runtime> for EvmDealWithFees {
	type LiquidityInfo = Option<Credit<AccountId, Balances>>;

	fn withdraw_fee(who: &H160, fee: U256) -> Result<Self::LiquidityInfo, pallet_evm::Error<Runtime>> {
		EVMFungibleAdapter::<Balances, DealWithFees>::withdraw_fee(who, fee)
	}

	fn correct_and_deposit_fee(
		who: &H160,
		corrected_fee: U256,
		base_fee: U256,
		already_withdrawn: Self::LiquidityInfo,
	) -> Self::LiquidityInfo {
		<EVMFungibleAdapter<Balances, DealWithFees> as OnChargeEVMTransaction<Runtime>>::correct_and_deposit_fee(
			who,
			corrected_fee,
			base_fee,
			already_withdrawn,
		)
	}

	fn pay_priority_fee(tip: Self::LiquidityInfo) {
		if let Some(tip) = tip {
			let dest = Authorship::author().unwrap_or_else(TreasuryAccount::get);
			let _ = <Balances as Balanced<AccountId>>::resolve(&dest, tip);
		}
	}
}

impl pallet_evm_chain_id::Config for Runtime {}

impl pallet_evm::Config for Runtime {
	type AccountProvider = pallet_evm::FrameSystemAccountProvider<Self>;
	type FeeCalculator = crate::BaseFee;
	type GasWeightMapping = pallet_evm::FixedGasWeightMapping<Self>;
	type WeightPerGas = WeightPerGas;
	// Use the Ethereum-side block hash mapping so that EVM `BLOCKHASH`
	// opcodes return the hashes recorded by `pallet-ethereum` for past
	// Ethereum-style blocks.
	type BlockHashMapping = pallet_ethereum::EthereumBlockHashMapping<Self>;
	// Substrate `AccountId32` cannot be safely lowered to a 20-byte address,
	// so the only direct EVM origins are root or `pallet-ethereum`'s own
	// self-contained calls (Step 4). All substrate-side users interact with
	// EVM contracts via the Ethereum extrinsic.
	type CallOrigin = EnsureAddressRoot<Self::AccountId>;
	type CreateOriginFilter = ();
	type CreateInnerOriginFilter = ();
	type WithdrawOrigin = EnsureAddressNever<Self::AccountId>;
	type AddressMapping = HashedAddressMapping<BlakeTwo256>;
	type Currency = Balances;
	type PrecompilesType = FrontierPrecompiles<Self>;
	type PrecompilesValue = PrecompilesValue;
	type ChainId = crate::EVMChainId;
	type BlockGasLimit = BlockGasLimit;
	type TransactionGasLimit = TransactionGasLimit;
	type Runner = pallet_evm::runner::stack::Runner<Self>;
	type OnChargeTransaction = EvmDealWithFees;
	type OnCreate = ();
	type FindAuthor = EvmFindAuthorZero;
	type GasLimitPovSizeRatio = GasLimitPovSizeRatio;
	type GasLimitStorageGrowthRatio = GasLimitStorageGrowthRatio;
	type Timestamp = Timestamp;
	type WeightInfo = crate::weights::pallet_evm::WeightInfo<Self>;
}

parameter_types! {
	pub const PostBlockAndTxnHashes: PostLogContent = PostLogContent::BlockAndTxnHashes;
	pub const AllowUnprotectedTxs: bool = false;
}

impl pallet_ethereum::Config for Runtime {
	type StateRoot = pallet_ethereum::IntermediateStateRoot<<Runtime as frame_system::Config>::Version>;
	type PostLogContent = PostBlockAndTxnHashes;
	type ExtraDataLength = ConstU32<30>;
	type AllowUnprotectedTxs = AllowUnprotectedTxs;
}

parameter_types! {
	/// Initial base fee. Ethereum's 1 gwei prices gas against an ether worth
	/// thousands. NUMN is not, so the unit runs three orders wider to keep block
	/// space priced above free.
	pub DefaultBaseFeePerGas: U256 = U256::from(1_000_000_000_000u64);
	pub DefaultElasticity: Permill = Permill::from_parts(125_000);
}

pub struct BaseFeeThreshold;
impl pallet_base_fee::BaseFeeThreshold for BaseFeeThreshold {
	fn lower() -> Permill {
		Permill::zero()
	}
	fn ideal() -> Permill {
		Permill::from_parts(500_000)
	}
	fn upper() -> Permill {
		Permill::from_parts(1_000_000)
	}
}

impl pallet_base_fee::Config for Runtime {
	type Threshold = BaseFeeThreshold;
	type DefaultBaseFeePerGas = DefaultBaseFeePerGas;
	type DefaultElasticity = DefaultElasticity;
}
