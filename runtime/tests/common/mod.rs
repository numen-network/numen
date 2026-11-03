//! Externalities builders shared by the runtime tests.

#![allow(dead_code)]

use codec::Encode;
use numen_runtime::{
	check_nonce::CheckNonceKeepSigned, genesis_config_presets as presets, Runtime, RuntimeCall,
	RuntimeGenesisConfig, SignedPayload, System, TxExtension, UncheckedExtrinsic,
};
use sp_core::Pair;
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{generic, generic::Era, BuildStorage, MultiAddress, MultiSignature};

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

/// The extensions a wallet signs with for an immortal transaction at `nonce`.
pub fn immortal_extension(nonce: u32) -> TxExtension {
	(
		frame_system::AuthorizeCall::<Runtime>::new(),
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(Era::Immortal),
		CheckNonceKeepSigned::<Runtime>::from(nonce),
		frame_system::CheckWeight::<Runtime>::new(),
		pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
		frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
		pallet_revive::evm::tx_extension::SetOrigin::<Runtime>::default(),
		frame_system::WeightReclaim::<Runtime>::new(),
	)
}

/// `call` signed by `signer` as an immortal transaction at `nonce`.
pub fn signed_immortal(signer: Sr25519Keyring, nonce: u32, call: RuntimeCall) -> UncheckedExtrinsic {
	let payload =
		SignedPayload::new(call, immortal_extension(nonce)).expect("implicit data resolves");
	let signature = payload.using_encoded(|bytes| signer.pair().sign(bytes));
	let (call, extension, _) = payload.deconstruct();
	generic::UncheckedExtrinsic::new_signed(
		call,
		MultiAddress::Id(signer.to_account_id()),
		MultiSignature::Sr25519(signature),
		extension,
	)
	.into()
}
