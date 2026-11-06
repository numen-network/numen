//! Externalities builders shared by the runtime tests.

#![allow(dead_code)]

use numen_runtime::{genesis_config_presets as presets, Runtime, RuntimeGenesisConfig, System};
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

/// Chain state as a node would build it from `preset`.
pub fn preset_ext(preset: &str) -> TestExternalities {
	let mut config = serde_json::to_value(RuntimeGenesisConfig::default()).unwrap();
	let patch = presets::get_preset(&preset.into()).expect("preset exists");
	sc_chain_spec::json_merge(&mut config, serde_json::from_slice(&patch).unwrap());
	let mut ext = TestExternalities::default();
	ext.execute_with(|| {
		frame_support::genesis_builder_helper::build_state::<RuntimeGenesisConfig>(
			serde_json::to_vec(&config).unwrap(),
		)
		.expect("preset builds");
	});
	ext
}
