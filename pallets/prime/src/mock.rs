use crate as pallet_prime;
use codec::Encode;
use frame_support::{derive_impl, parameter_types};
use sp_runtime::BuildStorage;
use sp_version::RuntimeVersion;

pub type AccountId = u64;

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
}

impl pallet_prime::Config for Test {
	type WeightInfo = ();
	type UpgradeOrigin = pallet_prime::EnsurePrime<Test>;
}

pub const PRIME: AccountId = 1;
pub const OTHER: AccountId = 2;

pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::<Test>::default()
		.build_storage()
		.unwrap();
	pallet_prime::GenesisConfig::<Test> { key: Some(PRIME) }
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
