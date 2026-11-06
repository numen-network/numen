//! Externalities builder shared by the runtime tests.

#![allow(dead_code)]

use numen_runtime::{Runtime, System};
use sp_io::TestExternalities;
use sp_runtime::BuildStorage;

/// Empty genesis with the block number advanced to 1 so events are recorded.
pub fn new_test_ext() -> TestExternalities {
	let storage = frame_system::GenesisConfig::<Runtime>::default()
		.build_storage()
		.expect("system genesis builds");
	let mut ext = TestExternalities::from(storage);
	ext.execute_with(|| System::set_block_number(1));
	ext
}
