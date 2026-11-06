//! The upgrade that hands Frontier's EVM state to pallet-revive, run on a chain
//! laid out the way Frontier left mainnet. Contracts keep code, storage,
//! balance and nonce and run on pallet-revive. Frontier accounts move to the
//! H160's new account, those with an EIP-7702 delegation too, and signers
//! stay pinned. The token proxy appears, block hashes carry over and
//! Frontier's own storage is gone.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{
	storage::unhashed,
	traits::{
		fungible::{Inspect, Mutate},
		OnRuntimeUpgrade,
	},
	weights::Weight,
	StorageHasher, Twox128,
};
use numen_runtime::{
	balances_erc20::{self, IBalancesErc20},
	configs::evm::EvmChainId,
	migrations::{frontier_storage, revive_storage, FrontierToRevive},
	AccountId, Balances, Revive, Runtime, RuntimeOrigin, System, UNIT,
};
use pallet_revive::{
	precompiles::alloy::sol_types::SolCall, AccountId32Mapper, AddressMapper, BytecodeType, Code,
	ExecConfig, TransactionLimits, H160, H256, U256,
};
use sp_core::Get;
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;

type EvmClaims = pallet_evm_claims::Pallet<Runtime>;

/// Returns storage slot 0.
const READER: [u8; 11] = hex_literal::hex!("60005460005260206000f3");

/// Stores its first calldata word in slot 0.
const WRITER: [u8; 7] = hex_literal::hex!("60003560005500");

/// One of the H160s the upgrade moves on its own.
const KNOWN: H160 = H160(hex_literal::hex!("b9cb2b8501933243a4ba5ad0f5fb3a256c8669fb"));

fn reader(index: u8) -> H160 {
	H160::repeat_byte(0xa0 + index)
}

fn writer() -> H160 {
	H160::repeat_byte(0xb0)
}

/// An account that set an EIP-7702 delegation under Frontier.
fn delegator() -> H160 {
	H160::repeat_byte(0xc0)
}

fn word(value: u8) -> H256 {
	H256::from_low_u64_be(value.into())
}

/// `balance` and `nonce` at the account Frontier derived for `address`.
fn frontier_account(address: H160, balance: u128, nonce: u32) {
	let legacy = EvmClaims::legacy_account(&address);
	System::inc_providers(&legacy);
	Balances::set_balance(&legacy, balance);
	frame_system::Account::<Runtime>::mutate(&legacy, |info| info.nonce = nonce);
}

fn frontier_contract(address: H160, code: &[u8], slots: &[(H256, H256)], balance: u128) {
	frontier_storage::AccountCodes::insert(address, code.to_vec());
	frontier_storage::AccountCodesMetadata::insert(
		address,
		(code.len() as u64, H256(sp_io::hashing::keccak_256(code))),
	);
	for (key, value) in slots {
		frontier_storage::AccountStorages::insert(address, key, value);
	}
	frontier_account(address, balance, 1);
}

/// Leaves a delegation to `writer()` at `delegator()`, whose Frontier account
/// holds a balance and signed five transactions.
fn delegate() {
	let mut code = hex_literal::hex!("ef0100").to_vec();
	code.extend_from_slice(writer().as_bytes());
	frontier_storage::AccountCodes::insert(delegator(), code);
	frontier_account(delegator(), 40 * UNIT, 5);
}

/// A chain at block 1,000 the way Frontier leaves it.
fn frontier_chain() -> TestExternalities {
	let mut ext = new_test_ext();
	ext.execute_with(|| {
		System::set_block_number(1_000);
		unhashed::put_raw(b":ethereum_schema", &[3]);
		let chain_id_key =
			[Twox128::hash(b"EVMChainId"), Twox128::hash(b"ChainId")].concat();
		unhashed::put_raw(&chain_id_key, &32_026u64.encode());
		for number in 600..1_000u32 {
			frontier_storage::BlockHash::insert(U256::from(number), H256::repeat_byte(number as u8));
		}
		unhashed::put_raw(&[Twox128::hash(b"Ethereum"), Twox128::hash(b"CurrentBlock")].concat(), &[1; 615]);
		unhashed::put_raw(&[Twox128::hash(b"BaseFee"), Twox128::hash(b"BaseFeePerGas")].concat(), &[2; 32]);

		frontier_contract(reader(0), &READER, &[(word(0), word(42))], 5 * UNIT);
		frontier_contract(reader(1), &READER, &[(word(0), word(7))], 0);
		frontier_contract(writer(), &WRITER, &[(word(0), word(9))], 0);
		frontier_account(KNOWN, 300 * UNIT, 2);

		let alice = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&alice, 1_000 * UNIT);
		frame_system::Account::<Runtime>::mutate(&alice, |info| info.nonce = 3);
	});
	ext
}

fn limits() -> TransactionLimits<Runtime> {
	TransactionLimits::WeightAndDeposit {
		weight_limit: Weight::from_parts(500_000_000_000, 5 * 1024 * 1024),
		deposit_limit: 10 * UNIT,
	}
}

fn call(dest: H160, data: Vec<u8>) -> Vec<u8> {
	let output = Revive::bare_call(
		RuntimeOrigin::signed(Sr25519Keyring::Alice.to_account_id()),
		dest,
		U256::zero(),
		limits(),
		data,
		&ExecConfig::new_substrate_tx(),
	)
	.result
	.expect("the call executes");
	assert!(!output.did_revert(), "the call reverted");
	output.data
}

fn upgrade() {
	FrontierToRevive::on_runtime_upgrade();
}

#[test]
fn contracts_keep_code_storage_balance_and_nonce() {
	frontier_chain().execute_with(|| {
		upgrade();

		assert_eq!(Revive::code(&reader(0)), READER.to_vec());
		assert_eq!(call(reader(0), Vec::new()), word(42).0.to_vec());
		assert_eq!(call(reader(1), Vec::new()), word(7).0.to_vec());
		assert_eq!(Revive::evm_balance(&reader(0)), U256::from(5 * UNIT));
		let account = AccountId32Mapper::<Runtime>::to_fallback_account_id(&reader(0));
		assert_eq!(System::account_nonce(&account), 1);
		assert!(!frame_system::Account::<Runtime>::contains_key(EvmClaims::legacy_account(
			&reader(0)
		)));
	});
}

/// The moved contracts carry no deposits, so pallet-revive refunds nothing for
/// their storage. A contract freeing or growing it still runs like any other.
#[test]
fn moved_storage_changes_without_deposit_trouble() {
	frontier_chain().execute_with(|| {
		upgrade();
		let info = pallet_revive::AccountInfo::<Runtime>::load_contract(&writer()).unwrap();
		assert_eq!((info.storage_items, info.storage_bytes), (1, 64));
		assert!(info.total_deposit() == 0);

		call(writer(), word(0).0.to_vec());
		assert_eq!(Revive::get_storage(writer(), word(0).0), Ok(None));
		call(writer(), word(5).0.to_vec());

		assert_eq!(Revive::get_storage(writer(), word(0).0), Ok(Some(word(5).0.to_vec())));
	});
}

#[test]
fn shared_code_is_stored_once_and_counted_per_contract() {
	frontier_chain().execute_with(|| {
		upgrade();

		let code_hash = pallet_revive::AccountInfo::<Runtime>::load_contract(&reader(0))
			.unwrap()
			.code_hash;
		let info = revive_storage::CodeInfoOf::get(code_hash).expect("the code is stored");
		assert_eq!(info.refcount, 2);
		assert_eq!(info.deposit, 0);
		assert_eq!(info.owner, Revive::account_id());
	});
}

/// The mirror the upgrade writes code records through decodes what
/// pallet-revive itself stores.
#[test]
fn the_code_record_mirror_reads_what_pallet_revive_writes() {
	common::preset_ext(sp_genesis_builder::DEV_RUNTIME_PRESET).execute_with(|| {
		System::set_block_number(1);
		let deployed = Revive::bare_instantiate(
			RuntimeOrigin::signed(Sr25519Keyring::Alice.to_account_id()),
			U256::zero(),
			limits(),
			Code::Upload(hex_literal::hex!("600b600c600039600b6000f360005460005260206000f3").to_vec()),
			Vec::new(),
			None,
			&ExecConfig::new_substrate_tx(),
		)
		.result
		.expect("the contract deploys");

		let code_hash =
			pallet_revive::AccountInfo::<Runtime>::load_contract(&deployed.addr).unwrap().code_hash;
		let info = revive_storage::CodeInfoOf::get(code_hash).expect("the record decodes");
		assert_eq!(info.refcount, 1);
		assert_eq!(info.code_len, READER.len() as u32);
		assert_eq!(info.code_type, BytecodeType::Evm);
		assert_eq!(info.owner, Sr25519Keyring::Alice.to_account_id());
	});
}

/// MetaMask shows the known H160 the balance and nonce its Frontier account
/// had, and nothing is left at the old account.
#[test]
fn a_known_account_moves_whole() {
	frontier_chain().execute_with(|| {
		upgrade();

		assert_eq!(Revive::evm_balance(&KNOWN), U256::from(300 * UNIT));
		assert_eq!(System::account_nonce(AccountId32Mapper::<Runtime>::to_account_id(&KNOWN)), 2);
		assert!(!frame_system::Account::<Runtime>::contains_key(EvmClaims::legacy_account(&KNOWN)));
	});
}

#[test]
fn a_delegation_moves_as_an_account() {
	frontier_chain().execute_with(|| {
		delegate();

		upgrade();

		assert!(Revive::code(&delegator()).is_empty(), "no contract sits at the delegator");
		assert_eq!(Revive::evm_balance(&delegator()), U256::from(40 * UNIT));
		assert_eq!(System::account_nonce(AccountId32Mapper::<Runtime>::to_account_id(&delegator())), 5);
		assert!(!frontier_storage::AccountCodes::contains_key(delegator()));
	});
}

#[test]
fn a_signer_gets_pinned_once() {
	frontier_chain().execute_with(|| {
		let alice: AccountId = Sr25519Keyring::Alice.to_account_id();
		let providers = frame_system::Account::<Runtime>::get(&alice).providers;

		upgrade();

		assert_eq!(frame_system::Account::<Runtime>::get(&alice).providers, providers + 1);
	});
}

#[test]
fn every_substrate_account_gets_its_own_h160() {
	frontier_chain().execute_with(|| {
		upgrade();

		for who in frame_system::Account::<Runtime>::iter_keys() {
			assert!(
				AccountId32Mapper::<Runtime>::is_eth_derived(&who) ||
					AccountId32Mapper::<Runtime>::is_mapped(&who),
				"{who:?} stays unmapped",
			);
		}
	});
}

#[test]
fn the_token_proxy_answers_at_0x0802() {
	frontier_chain().execute_with(|| {
		upgrade();

		let data = call(balances_erc20::ADDRESS, IBalancesErc20::nameCall {}.abi_encode());
		assert_eq!(IBalancesErc20::nameCall::abi_decode_returns(&data).unwrap(), "Numen");
	});
}

/// The hashes of the last 256 Ethereum blocks carry over. `BLOCKHASH` and the
/// parent of the next Ethereum block read them from there.
#[test]
fn the_last_256_block_hashes_carry_over() {
	frontier_chain().execute_with(|| {
		upgrade();

		for number in 744..1_000u32 {
			assert_eq!(revive_storage::BlockHash::get(number), H256::repeat_byte(number as u8));
		}
		assert_eq!(revive_storage::BlockHash::get(700), H256::zero());
	});
}

#[test]
fn issuance_grows_only_by_the_deposits_the_new_accounts_need() {
	frontier_chain().execute_with(|| {
		let total = Balances::total_issuance();
		let active = Balances::active_issuance();

		upgrade();

		// Three contracts, the token proxy, the known account and the pallet's
		// own account each get one existential deposit.
		assert_eq!(Balances::total_issuance(), total + 6);
		assert_eq!(Balances::active_issuance(), active);
	});
}

/// Frontier's pallets leave nothing behind, while the chain id and the token
/// allowances stay where everything reads them.
#[test]
fn frontier_storage_is_gone_and_shared_state_stays() {
	frontier_chain().execute_with(|| {
		let owner = H160::repeat_byte(1);
		let spender = H160::repeat_byte(2);
		balances_erc20::Allowances::insert(owner, spender, U256::from(9));

		upgrade();

		for pallet in [&b"Ethereum"[..], b"BaseFee", b"EVM"] {
			assert!(!unhashed::contains_prefixed_key(&Twox128::hash(pallet)), "{pallet:?} left");
		}
		assert!(unhashed::get_raw(b":ethereum_schema").is_none());
		assert_eq!(EvmChainId::get(), 32_026);
		assert_eq!(balances_erc20::Allowances::get(owner, spender), U256::from(9));
	});
}

#[test]
fn a_second_run_changes_nothing() {
	frontier_chain().execute_with(|| {
		upgrade();
		let root = sp_io::storage::root(sp_runtime::StateVersion::V1);

		upgrade();

		assert_eq!(sp_io::storage::root(sp_runtime::StateVersion::V1), root);
	});
}

/// A contract pallet-revive cannot take stays in Frontier's storage for a later
/// upgrade. Everything else moves regardless.
#[test]
fn a_contract_that_cannot_move_stays_and_the_rest_goes_ahead() {
	frontier_chain().execute_with(|| {
		let oversized = H160::repeat_byte(0xc0);
		frontier_contract(oversized, &vec![0x5b; 24_577], &[(word(1), word(1))], UNIT);

		upgrade();

		assert!(frontier_storage::AccountCodes::contains_key(oversized));
		assert_eq!(frontier_storage::AccountStorages::get(oversized, word(1)), word(1));
		assert_eq!(Balances::balance(&EvmClaims::legacy_account(&oversized)), UNIT);
		assert_eq!(call(reader(0), Vec::new()), word(42).0.to_vec());
		assert!(unhashed::contains_prefixed_key(&Twox128::hash(b"EVM")));
	});
}

/// The try-runtime checks hold on this chain, with a delegation among the
/// codes.
#[cfg(feature = "try-runtime")]
#[test]
fn the_upgrade_checks_hold_with_a_delegation_among_the_codes() {
	frontier_chain().execute_with(|| {
		delegate();
		let snapshot = FrontierToRevive::pre_upgrade().expect("the snapshot is taken");

		upgrade();

		FrontierToRevive::post_upgrade(snapshot).expect("the checks hold");
	});
}
