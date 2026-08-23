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
		)
	]
}

fn testnet_validators() -> Vec<(AccountId, GrandpaId, ImOnlineId)> {
	vec![
		(
			hex!("d691b51d8e033fee40766cd7ed830a953689a128aa76a084adcfe85ce169b251").into(),
			hex!("963234b3d31612fe465adf6089b9e6cda6fa2e9489f0bc39fe5cf2cbe53525ad").unchecked_into(),
			hex!("62ed90b1060d4acc665e1bca4d80a2c646dd2160e5252f3ca7c69485768b3a17").unchecked_into(),
		),
	]
}

fn live_validators() -> Vec<(AccountId, GrandpaId, ImOnlineId)> {
	vec![
		(
			hex!("0c75ace277b402399cf8fb811b871169e5d675f9f7c5c9a72a72c4bb8044d917").into(),
			hex!("442e9503c1fc1289fa31770fd14735c6c109ee72eec64aa7af63fe200dfcca2c").unchecked_into(),
			hex!("001ae5824a816a9f2e6b8758d906b624f66a835fd1473f93a8f4cb0057ed6558").unchecked_into(),
		),
		(
			hex!("0c75ace1a6f6b7857e03f08e170590ed34fbded0b415d37de2dda6c2fa2bf85d").into(),
			hex!("1e2de688a48e6601f348d2a96e5f28b760e1b0a3f5437db5600d6d8fb4f59c15").unchecked_into(),
			hex!("9a6a8618d85624e1bc9bc28f02b7b2911d310881b5a7e59da739fcf4b0b8c02c").unchecked_into(),
		),
		(
			hex!("0c75ace1f15ade9f7c63fbe5cf37982d9699a2667e14c789f37b5b720cba8e10").into(),
			hex!("2fb4a53bcbc662cf9fbf572644fe8cb510259bea5e30614c2349da84bf6e8731").unchecked_into(),
			hex!("18ad57728b13145ade005d1320dc17e66c6d79a1ce438697428766e15972fa5b").unchecked_into(),
		),
		(
			hex!("0c75ace1f34508360008ad6b21df5f69412786019ce84847c8609f239e4d6123").into(),
			hex!("b55955ce16778ab577f80af54d4e5258d57dd674fc9f64fd782fae9512e9f954").unchecked_into(),
			hex!("b2d1964319a98bb8d621758947737db7425dca32a83aeb2055799e6b4a73ed49").unchecked_into(),
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

// nu7PrimeGGWHhqsFvKxLbmCsudaCDMDYaKwiDVR46k3AHUtYk
const PRIME: [u8; 32] = hex!("d23460fba7462ff9493b18c9974274af46e6c23ac219cfb0f38930b36275576a");


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
		Some(PRIME.into()),
		testnet_validators(),
		TEST_EVM_CHAIN_ID,
		BTreeMap::new(),
		INITIAL_DIFFICULTY.into()
	)
}

pub fn mainnet_config_genesis() -> Value {
	genesis_patch(
		live_balances(),
		Some(PRIME.into()),
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
