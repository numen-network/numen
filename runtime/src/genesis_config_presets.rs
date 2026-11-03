use crate::{
	configs::{evm::DEV_CHAIN_ID, FEE_ANCHOR},
	AccountId, BalancesConfig, DifficultyConfig, EVMChainIdConfig,
	GenesisRegistrarsConfig, PrimeConfig, Runtime, RuntimeGenesisConfig, SessionConfig,
	SessionKeys, TransactionPaymentConfig, ValidatorConfig, UNIT,
};
use alloc::{vec, vec::Vec};
use frame_support::build_struct_json_patch;
use pallet_transaction_payment::Multiplier;
use hex_literal::hex;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use pallet_revive::{AccountId32Mapper, AddressMapper};
use serde_json::Value;
use sp_consensus_grandpa::AuthorityId as GrandpaId;
use sp_core::{crypto::UncheckedInto, H160, U256};
use sp_genesis_builder::{self, PresetId};
use sp_runtime::FixedPointNumber;
use sp_keyring::{Ed25519Keyring, Sr25519Keyring};

const GENESIS_TREASURY_ISSUANCE: u128 = 550_000_000 * UNIT;
const GENESIS_AIRDROP_ISSUANCE: u128 = 50_000_000 * UNIT;

const INITIAL_DIFFICULTY: u32 = 1_000;
const MAIN_INITIAL_DIFFICULTY: u32 = 1_000_000;

const DEV_ACCOUNT_BALANCE: u128 = 1_000_000 * UNIT;

const TEST_EVM_CHAIN_ID: u64 = 320261;
const MAIN_EVM_CHAIN_ID: u64 = 32026;

/// Alith, Baltathar, Charleth, Dorothy, Ethan and Faith, the dev keys EVM
/// tooling ships with.
const DEV_EVM_ACCOUNTS: [[u8; 20]; 6] = [
	hex!("f24ff3a9cf04c71dbc94d0b566f7a27b94566cac"),
	hex!("3cd0a705a2dc65e5b1e1205896baa2be8a07c6e0"),
	hex!("798d4ba9baf0064ec19eb4f0a1a45785ae9d6dfc"),
	hex!("773539d4ac0e786233d90a233654ccee26a613d9"),
	hex!("ff64d3f6efe2317ee2807d223a0bdc4c0c49dfdb"),
	hex!("c0f0f4ab324c46e55d02d0033343b4be8a55532d"),
];

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
	.into_iter()
	.chain(DEV_EVM_ACCOUNTS.iter().map(|address| {
		(AccountId32Mapper::<Runtime>::to_fallback_account_id(&H160(*address)), DEV_ACCOUNT_BALANCE)
	}))
	.collect()
}

fn live_balances() -> Vec<(AccountId, u128)> {
	vec![
		(crate::configs::TreasuryAccount::get(), GENESIS_TREASURY_ISSUANCE),

		// nu5DropA1BwvKhPMDLrMXNFCmo4B9PK71iuZEMxESSJGvoyMg
		(hex!("721bc027868036fd2aecafd08b6942f60da6070992a2564822ee3b3307a5ca5b").into(), GENESIS_AIRDROP_ISSUANCE)
	]
}

fn dev_registrars() -> Vec<AccountId> {
	vec![
		Sr25519Keyring::Alice.to_account_id()
	]
}

fn live_registrars() -> Vec<AccountId> {
	vec![
		// nu7iBot1ZuaFNt6wh4CQ8B8yh82Q6PsgJF7N7gfCGnoLLxZwv
		hex!("e02f0d696b3b9eecad3e7bf0e651a7c4d8ec8925a8ae9662a268655d1da26546").into()
	]
}

// nu7PrimeGGWHhqsFvKxLbmCsudaCDMDYaKwiDVR46k3AHUtYk
const PRIME: [u8; 32] = hex!("d23460fba7462ff9493b18c9974274af46e6c23ac219cfb0f38930b36275576a");


fn genesis_patch(
	balances: Vec<(AccountId, u128)>,
	prime_key: Option<AccountId>,
	registrars: Vec<AccountId>,
	validators: Vec<(AccountId, GrandpaId, ImOnlineId)>,
	chain_id: u64,
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
		genesis_registrars: GenesisRegistrarsConfig { registrars },
		difficulty: DifficultyConfig { initial_difficulty },
		session: SessionConfig { keys: session_keys },
		validator: ValidatorConfig {
			initial_validators: validators.iter().map(|(a, _, _)| a.clone()).collect::<Vec<_>>(),
			..Default::default()
		},
		transaction_payment: TransactionPaymentConfig {
			multiplier: Multiplier::saturating_from_integer(FEE_ANCHOR),
		},
		evm_chain_id: EVMChainIdConfig { chain_id },
	})
}


pub fn development_config_genesis() -> Value {
	genesis_patch(
		dev_balances(),
		Some(Sr25519Keyring::Alice.to_account_id()),
		dev_registrars(),
		dev_validators(),
		DEV_CHAIN_ID,
		INITIAL_DIFFICULTY.into()
	)
}

pub fn local_config_genesis() -> Value {
	genesis_patch(
		dev_balances(),
		Some(Sr25519Keyring::Alice.to_account_id()),
		dev_registrars(),
		dev_validators(),
		DEV_CHAIN_ID,
		INITIAL_DIFFICULTY.into()
	)
}

pub fn integration_config_genesis() -> Value {
	genesis_patch(
		dev_balances(),
		Some(Sr25519Keyring::Alice.to_account_id()),
		dev_registrars(),
		dev_validators(),
		DEV_CHAIN_ID,
		INITIAL_DIFFICULTY.into()
	)
}

pub fn testnet_config_genesis() -> Value {
	genesis_patch(
		live_balances(),
		Some(PRIME.into()),
		live_registrars(),
		testnet_validators(),
		TEST_EVM_CHAIN_ID,
		INITIAL_DIFFICULTY.into()
	)
}

pub fn mainnet_config_genesis() -> Value {
	genesis_patch(
		live_balances(),
		Some(PRIME.into()),
		live_registrars(),
		live_validators(),
		MAIN_EVM_CHAIN_ID,
		MAIN_INITIAL_DIFFICULTY.into()
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
