use crate::{
	AccountId, BalancesConfig, DifficultyConfig, EVMChainIdConfig, EVMConfig, PrimeConfig,
	RuntimeGenesisConfig, SessionConfig, SessionKeys, ValidatorConfig, UNIT,
};
use alloc::{collections::BTreeMap, vec, vec::Vec};
use fp_evm::GenesisAccount;
use frame_support::build_struct_json_patch;
use hex_literal::hex;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use serde_json::Value;
use sp_consensus_grandpa::AuthorityId as GrandpaId;
use sp_core::{crypto::UncheckedInto, H160, U256};
use sp_genesis_builder::{self, PresetId};
use sp_keyring::{Ed25519Keyring, Sr25519Keyring};

const GENESIS_TREASURY_ISSUANCE: u128 = 550_000_000 * UNIT;
const GENESIS_AIRDROP_ISSUANCE: u128 = 50_000_000 * UNIT;

const INITIAL_DIFFICULTY: u32 = 1_000;

const DEV_EVM_ACCOUNT_BALANCE: u128 = 1_000_000 * UNIT;
const DEV_ACCOUNT_BALANCE: u128 = 1_000_000 * UNIT;

const DEV_EVM_CHAIN_ID: u64 = 320262;
const TEST_EVM_CHAIN_ID: u64 = 320261;
const MAIN_EVM_CHAIN_ID: u64 = 32026;

fn dev_evm_accounts() -> BTreeMap<H160, GenesisAccount> {
	let balance = U256::from(DEV_EVM_ACCOUNT_BALANCE);
	let make = |bytes: [u8; 20]| {
		(
			H160::from(bytes),
			GenesisAccount {
				nonce: U256::zero(),
				balance,
				storage: Default::default(),
				code: Default::default(),
			},
		)
	};
	[
		make([0xf2, 0x4f, 0xf3, 0xa9, 0xcf, 0x04, 0xc7, 0x1d, 0xbc, 0x94, 0xd0, 0xb5, 0x66, 0xf7, 0xa2, 0x7b, 0x94, 0x56, 0x6c, 0xac]), // Alith
		make([0x3c, 0xd0, 0xa7, 0x05, 0xa2, 0xdc, 0x65, 0xe5, 0xb1, 0xe1, 0x20, 0x58, 0x96, 0xba, 0xa2, 0xbe, 0x8a, 0x07, 0xc6, 0xe0]), // Baltathar
		make([0x79, 0x8d, 0x4b, 0xa9, 0xba, 0xf0, 0x06, 0x4e, 0xc1, 0x9e, 0xb4, 0xf0, 0xa1, 0xa4, 0x57, 0x85, 0xae, 0x9d, 0x6d, 0xfc]), // Charleth
		make([0x77, 0x35, 0x39, 0xd4, 0xac, 0x0e, 0x78, 0x62, 0x33, 0xd9, 0x0a, 0x23, 0x36, 0x54, 0xcc, 0xee, 0x26, 0xa6, 0x13, 0xd9]), // Dorothy
		make([0xff, 0x64, 0xd3, 0xf6, 0xef, 0xe2, 0x31, 0x7e, 0xe2, 0x80, 0x7d, 0x22, 0x3a, 0x0b, 0xdc, 0x4c, 0x0c, 0x49, 0xdf, 0xdb]), // Ethan
		make([0xc0, 0xf0, 0xf4, 0xab, 0x32, 0x4c, 0x46, 0xe5, 0x5d, 0x02, 0xd0, 0x03, 0x33, 0x43, 0xb4, 0xbe, 0x8a, 0x55, 0x53, 0x2d]), // Faith
	]
	.into_iter()
	.collect()
}

fn dev_validators() -> Vec<(AccountId, GrandpaId, ImOnlineId)> {
	vec![
		(
			Sr25519Keyring::Alice.to_account_id(),
			Ed25519Keyring::Alice.public().into(),
			Sr25519Keyring::Alice.public().into(),
		),
		(
			Sr25519Keyring::Bob.to_account_id(),
			Ed25519Keyring::Bob.public().into(),
			Sr25519Keyring::Bob.public().into(),
		),
		(
			Sr25519Keyring::Charlie.to_account_id(),
			Ed25519Keyring::Charlie.public().into(),
			Sr25519Keyring::Charlie.public().into(),
		),
	]
}

fn live_validators() -> Vec<(AccountId, GrandpaId, ImOnlineId)> {
	vec![
		(
			hex!("7ebc23de675cd320952153f65eed8636ede3ee914d38d86a03b8690c5ef87745").into(),
			hex!("23209a84d105da8b36c9a90c00d92f518f2599d7ebe502a393ad4e22c5d4839a").unchecked_into(),
			hex!("dc5fa9c5793d7543808c84773f3cbce7928ccf4ce94568c7d4ab7a0e53de5037").unchecked_into(),
		),
		(
			hex!("f83e5c47238ae444ef3165741b0c2a26a15bfb910655376680dc86d47032ee71").into(),
			hex!("fdeb4cc2c3ce7049ffe91c86e4c777d5d381cf4a66a794d254175c4f68d51f47").unchecked_into(),
			hex!("9c3c925508d53f37ad9c9db0807f2cbd283d02ef34481d90113c2ec9ec195662").unchecked_into(),
		),
		(
			hex!("a08b338e366fdd4d7a4e66cd6ff1c8fe3f8d8b58f72ea980938612df2a12cb2f").into(),
			hex!("98c5493056057f89d374b64f5dc476f796cf64901d3f625e870c406d610f6fcb").unchecked_into(),
			hex!("9ac75734257b6aaf82e526030783b3f4fa9823a22c5fcf23b5115642ad12a871").unchecked_into(),
		),
	]
}

fn dev_balances() -> Vec<(AccountId, u128)> {
	vec![
		(Sr25519Keyring::Alice  .to_account_id(), DEV_ACCOUNT_BALANCE),
		(Sr25519Keyring::Bob    .to_account_id(), DEV_ACCOUNT_BALANCE),
		(Sr25519Keyring::Charlie.to_account_id(), DEV_ACCOUNT_BALANCE),
		(Sr25519Keyring::Dave   .to_account_id(), DEV_ACCOUNT_BALANCE),
		(Sr25519Keyring::Eve    .to_account_id(), DEV_ACCOUNT_BALANCE),
		(Sr25519Keyring::Ferdie .to_account_id(), DEV_ACCOUNT_BALANCE),
		(crate::configs::TreasuryAccount::get(), GENESIS_TREASURY_ISSUANCE)
	]
}

fn live_balances() -> Vec<(AccountId, u128)> {
	vec![
		(crate::configs::TreasuryAccount::get(), GENESIS_TREASURY_ISSUANCE),

		// nu5DropA1BwvKhPMDLrMXNFCmo4B9PK71iuZEMxESSJGvoyMg
		(hex!("721bc027868036fd2aecafd08b6942f60da6070992a2564822ee3b3307a5ca5b").into(), GENESIS_AIRDROP_ISSUANCE)
	]
}

// nu7bkLdXi7aBVP8phwhrm3Js8PgNjseDbaj5XPuzfkQhgDzYT
const PRIME_MULTISIG: [u8; 32] = hex!("db45d74546b00d6efd137e9a2ea63a3b1595bad6aa3407370eedf43d59fb0b5a");


fn genesis_patch(
	balances: Vec<(AccountId, u128)>,
	prime_key: Option<AccountId>,
	validators: Vec<(AccountId, GrandpaId, ImOnlineId)>,
	chain_id: u64,
	evm_accounts: BTreeMap<H160, GenesisAccount>,
	initial_difficulty: U256
) -> Value {

	let session_keys = validators
		.iter()
		.cloned()
		.map(|(account, grandpa, im_online)| {
			(account.clone(), account, SessionKeys { grandpa, im_online })
		})
		.collect();

	build_struct_json_patch!(RuntimeGenesisConfig {
		balances: BalancesConfig { balances },
		prime: PrimeConfig { key: prime_key },
		difficulty: DifficultyConfig { initial_difficulty },
		session: SessionConfig { keys: session_keys },
		validator: ValidatorConfig {
			initial_validators: validators.iter().map(|(a, _, _)| a.clone()).collect::<Vec<_>>(),
			..Default::default()
		},
		evm_chain_id: EVMChainIdConfig { chain_id, ..Default::default() },
		evm: EVMConfig { accounts: evm_accounts, ..Default::default() },
	})
}


pub fn development_config_genesis() -> Value {
	genesis_patch(
		dev_balances(),
		Some(Sr25519Keyring::Alice.to_account_id()),
		dev_validators(),
		DEV_EVM_CHAIN_ID,
		dev_evm_accounts(),
		INITIAL_DIFFICULTY.into()
	)
}

pub fn local_config_genesis() -> Value {
	genesis_patch(
		dev_balances(),
		Some(Sr25519Keyring::Alice.to_account_id()),
		dev_validators(),
		DEV_EVM_CHAIN_ID,
		dev_evm_accounts(),
		INITIAL_DIFFICULTY.into()
	)
}

pub fn integration_config_genesis() -> Value {
	genesis_patch(
		dev_balances(),
		Some(Sr25519Keyring::Alice.to_account_id()),
		dev_validators(),
		DEV_EVM_CHAIN_ID,
		dev_evm_accounts(),
		INITIAL_DIFFICULTY.into()
	)
}

pub fn testnet_config_genesis() -> Value {
	genesis_patch(
		live_balances(),
		Some(PRIME_MULTISIG.into()),
		live_validators(),
		TEST_EVM_CHAIN_ID,
		BTreeMap::new(),
		INITIAL_DIFFICULTY.into()
	)
}

pub fn mainnet_config_genesis() -> Value {
	genesis_patch(
		live_balances(),
		Some(PRIME_MULTISIG.into()),
		live_validators(),
		MAIN_EVM_CHAIN_ID,
		BTreeMap::new(),
		INITIAL_DIFFICULTY.into()
	)
}


pub const INTEGRATION_RUNTIME_PRESET: &str = "integration";
pub const TESTNET_RUNTIME_PRESET: &str = "testnet";
pub const MAINNET_RUNTIME_PRESET: &str = "mainnet";

/// Provides the JSON representation of predefined genesis config for given `id`.
pub fn get_preset(id: &PresetId) -> Option<Vec<u8>> {
	let patch = match id.as_ref() {
		sp_genesis_builder::DEV_RUNTIME_PRESET => development_config_genesis(),
		sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET => local_config_genesis(),
		INTEGRATION_RUNTIME_PRESET => integration_config_genesis(),
		TESTNET_RUNTIME_PRESET => testnet_config_genesis(),
		MAINNET_RUNTIME_PRESET => mainnet_config_genesis(),
		_ => return None,
	};
	Some(
		serde_json::to_string(&patch)
			.expect("serialization to json is expected to work. qed.")
			.into_bytes(),
	)
}

/// List of supported presets.
pub fn preset_names() -> Vec<PresetId> {
	vec![
		PresetId::from(sp_genesis_builder::DEV_RUNTIME_PRESET),
		PresetId::from(sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET),
		PresetId::from(INTEGRATION_RUNTIME_PRESET),
		PresetId::from(TESTNET_RUNTIME_PRESET),
		PresetId::from(MAINNET_RUNTIME_PRESET),
	]
}
