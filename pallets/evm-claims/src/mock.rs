use crate as pallet_evm_claims;
use frame_support::{
	derive_impl, parameter_types,
	traits::{fungible::Credit, ConstU128, Imbalance, OnUnbalanced, WithdrawReasons},
};
use sp_runtime::{
	traits::{ConvertInto, IdentityLookup},
	AccountId32, BuildStorage,
};

pub type Balance = u128;

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
	pub struct Test;

	#[runtime::pallet_index(0)]
	pub type System = frame_system::Pallet<Test>;

	#[runtime::pallet_index(1)]
	pub type Balances = pallet_balances::Pallet<Test>;

	#[runtime::pallet_index(2)]
	pub type Vesting = pallet_vesting::Pallet<Test>;

	#[runtime::pallet_index(3)]
	pub type EvmClaims = pallet_evm_claims::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = frame_system::mocking::MockBlock<Test>;
	type AccountId = AccountId32;
	type Lookup = IdentityLookup<AccountId32>;
	type AccountData = pallet_balances::AccountData<Balance>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type AccountStore = System;
	type Balance = Balance;
	type ExistentialDeposit = ConstU128<1>;
}

parameter_types! {
	pub UnvestedFundsAllowedWithdrawReasons: WithdrawReasons =
		WithdrawReasons::except(WithdrawReasons::TRANSFER | WithdrawReasons::RESERVE);
}

impl pallet_vesting::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type BlockNumberToBalance = ConvertInto;
	type MinVestedTransfer = ConstU128<1>;
	type UnvestedFundsAllowedWithdrawReasons = UnvestedFundsAllowedWithdrawReasons;
	type BlockNumberProvider = System;
	type WeightInfo = ();
	const MAX_VESTING_SCHEDULES: u32 = 28;
}

parameter_types! {
	pub const ClaimFee: Balance = 7;
	/// Fees handed over so far.
	pub static FeesTaken: Balance = 0;
}

pub struct TakeFees;

impl OnUnbalanced<Credit<AccountId32, Balances>> for TakeFees {
	fn on_nonzero_unbalanced(fee: Credit<AccountId32, Balances>) {
		FeesTaken::mutate(|taken| *taken += fee.peek());
	}
}

impl pallet_evm_claims::Config for Test {
	type ClaimFee = ClaimFee;
	type OnClaimFee = TakeFees;
	type WeightInfo = ();
}

pub fn new_test_ext() -> sp_io::TestExternalities {
	let storage = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
	let mut ext: sp_io::TestExternalities = storage.into();
	ext.execute_with(|| {
		System::set_block_number(1);
		FeesTaken::set(0);
	});
	ext
}
