use crate as pallet_prime;
use codec::Encode;
use frame_support::{derive_impl, parameter_types, traits::{ConstU32, ConstU64}};
use frame_system::EnsureRoot;
use sp_runtime::{
	testing::{TestSignature, UintAuthorityId},
	BuildStorage,
};
use sp_version::RuntimeVersion;

pub type AccountId = u64;
pub type Balance = u64;

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
	pub type Prime = pallet_prime::Pallet<Test>;

	#[runtime::pallet_index(2)]
	pub type Balances = pallet_balances::Pallet<Test>;

	#[runtime::pallet_index(3)]
	pub type Identity = pallet_identity::Pallet<Test>;
}

parameter_types! {
	pub Version: RuntimeVersion = RuntimeVersion {
		spec_name: alloc::borrow::Cow::Borrowed("test"),
		spec_version: 1,
		..Default::default()
	};
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = frame_system::mocking::MockBlock<Test>;
	type AccountId = AccountId;
	type Lookup = sp_runtime::traits::IdentityLookup<AccountId>;
	type Version = Version;
	type AccountData = pallet_balances::AccountData<Balance>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type AccountStore = System;
}

impl pallet_identity::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type BasicDeposit = ConstU64<10>;
	type ByteDeposit = ConstU64<1>;
	type UsernameDeposit = ConstU64<1>;
	type SubAccountDeposit = ConstU64<10>;
	type MaxSubAccounts = ConstU32<2>;
	type IdentityInformation = pallet_identity::legacy::IdentityInfo<ConstU32<2>>;
	type MaxRegistrars = ConstU32<4>;
	type Slashed = ();
	type ForceOrigin = EnsureRoot<AccountId>;
	type RegistrarOrigin = EnsureRoot<AccountId>;
	type OffchainSignature = TestSignature;
	type SigningPublicKey = UintAuthorityId;
	type UsernameAuthorityOrigin = EnsureRoot<AccountId>;
	type PendingUsernameExpiration = ConstU64<100>;
	type UsernameGracePeriod = ConstU64<2>;
	type MaxSuffixLength = ConstU32<7>;
	type MaxUsernameLength = ConstU32<32>;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = UsernameSigner;
	type WeightInfo = ();
}

impl pallet_prime::Config for Test {
	type WeightInfo = ();
}

/// The identity pallet needs a signer a benchmark can drive for username
/// claims. This mock runs on test signatures rather than real crypto.
#[cfg(feature = "runtime-benchmarks")]
pub struct UsernameSigner;

#[cfg(feature = "runtime-benchmarks")]
impl pallet_identity::BenchmarkHelper<UintAuthorityId, TestSignature> for UsernameSigner {
	fn sign_message(message: &[u8]) -> (UintAuthorityId, TestSignature) {
		(
			UintAuthorityId(SUBJECT),
			TestSignature(SUBJECT, message.to_vec()),
		)
	}
}

pub const PRIME: AccountId = 1;
pub const OTHER: AccountId = 2;
pub const REGISTRAR: AccountId = 3;
pub const SUBJECT: AccountId = 4;

pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::<Test>::default()
		.build_storage()
		.unwrap();
	pallet_prime::GenesisConfig::<Test> { key: Some(PRIME) }
		.assimilate_storage(&mut t)
		.unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances: vec![(SUBJECT, 1_000)],
		..Default::default()
	}
	.assimilate_storage(&mut t)
	.unwrap();
	let mut ext: sp_io::TestExternalities = t.into();
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Version probe stub fed to the externalities in place of a wasm executor.
struct ReadRuntimeVersion(Vec<u8>);

impl sp_core::traits::ReadRuntimeVersion for ReadRuntimeVersion {
	fn read_runtime_version(
		&self,
		_wasm_code: &[u8],
		_ext: &mut dyn sp_externalities::Externalities,
	) -> Result<Vec<u8>, String> {
		Ok(self.0.clone())
	}
}

/// Externalities whose version probe reports `version` for any code blob.
pub fn new_test_ext_with_version(version: RuntimeVersion) -> sp_io::TestExternalities {
	let mut ext = new_test_ext();
	ext.register_extension(sp_core::traits::ReadRuntimeVersionExt::new(ReadRuntimeVersion(
		version.encode(),
	)));
	ext
}
