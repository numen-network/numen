use crate as pallet_validator;
use crate::SessionInterface;
use frame_support::{
	derive_impl, parameter_types,
	traits::{ConstU128, ConstU32, ConstU64, Contains, LockIdentifier, Hooks},
};
use sp_runtime::BuildStorage;

pub type AccountId = u64;
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
	pub type Validator = pallet_validator::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = frame_system::mocking::MockBlock<Test>;
	type AccountId = AccountId;
	type AccountData = pallet_balances::AccountData<Balance>;
	type Lookup = sp_runtime::traits::IdentityLookup<AccountId>;
}

impl pallet_balances::Config for Test {
	type MaxLocks = ConstU32<10>;
	type MaxReserves = ();
	type ReserveIdentifier = [u8; 8];
	type Balance = Balance;
	type RuntimeEvent = RuntimeEvent;
	type DustRemoval = ();
	type ExistentialDeposit = ConstU128<1>;
	type AccountStore = System;
	type WeightInfo = ();
	type RuntimeHoldReason = RuntimeHoldReason;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	type DoneSlashHandler = ();
}

parameter_types! {
	pub const TestLockId: LockIdentifier = *b"validatr";
	pub static MissingSessionKeys: alloc::collections::BTreeSet<AccountId> =
		alloc::collections::BTreeSet::new();
}

/// Mock `SessionInterface` whose `has_keys` answer is driven by the
/// `MissingSessionKeys` static, allowing tests to flip the response per case.
pub struct MockSession;
impl SessionInterface<AccountId> for MockSession {
	fn has_keys(who: &AccountId) -> bool {
		!MissingSessionKeys::get().contains(who)
	}
}

parameter_types! {
	pub static UnqualifiedIdentities: alloc::collections::BTreeSet<AccountId> =
		alloc::collections::BTreeSet::new();
}

/// Mock identity gate whose verdict is driven by the `UnqualifiedIdentities`
/// static, allowing tests to flip the response per case.
pub struct MockIdentityGate;
impl Contains<AccountId> for MockIdentityGate {
	fn contains(who: &AccountId) -> bool {
		!UnqualifiedIdentities::get().contains(who)
	}
}

impl pallet_validator::Config for Test {
	type Currency = Balances;
	type SessionInterface = MockSession;
	type LockAmount = ConstU128<1_000>;
	type ExemptOrigin = frame_system::EnsureRoot<AccountId>;
	type IdentityGate = MockIdentityGate;
	type LockDuration = ConstU64<10>;
	type SessionPeriod = ConstU64<5>;
	type SessionOffset = ConstU64<0>;
	type LockId = TestLockId;
	type MaxValidators = ConstU32<3>;
	type RenewInterval = ConstU64<5>;
	type OfflineThreshold = ConstU32<2>;
	type RejoinCooldownPeriod = ConstU64<20>;
	type WeightInfo = ();
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = ();
}

pub const ALICE: AccountId = 1;
pub const BOB: AccountId = 2;
pub const CHARLIE: AccountId = 3;
pub const EXEMPT: AccountId = 4;

pub fn new_test_ext(balances: Vec<(AccountId, Balance)>) -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::<Test>::default()
		.build_storage()
		.unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances,
		..Default::default()
	}
	.assimilate_storage(&mut t)
	.unwrap();
	let mut ext: sp_io::TestExternalities = t.into();
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Advance the chain to `target` block, calling `on_initialize` for each new block.
pub fn run_to_block(target: u64) {
    while System::block_number() < target {
		System::set_block_number(System::block_number() + 1);
    	Validator::on_initialize(System::block_number());
    }
}

pub fn new_session(index: u32) -> Option<alloc::vec::Vec<AccountId>> {
	let vec = 
		<crate::Pallet<Test> as pallet_session::historical::SessionManager<AccountId, (),>>
		::new_session(index);
	vec.map(|v| v.into_iter().map(|(account, _)| account).collect())
}