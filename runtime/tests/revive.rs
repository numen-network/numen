//! pallet-revive wired into the runtime. A contract deploys under the one plank
//! existential deposit, an Ethereum transaction passes the pool and lands in a
//! block, a native transfer reports no more gas than wallets assume, and the
//! gas price sits at the fee anchor times the gas scale. Ethereum fees split
//! between the author and the treasury, and a new storage slot holds what an
//! SSTORE cost under Frontier. A drained Ethereum signer keeps its nonce, a
//! reaped account keeps its H160, and every network keeps its own chain id.

mod common;

use codec::Encode;
use common::preset_ext;
use frame_support::{
	storage::{unhashed, with_transaction},
	traits::{
		fungible::Mutate,
		tokens::{Fortitude, Precision, Preservation},
		Get,
	},
	weights::Weight,
};
use numen_runtime::{
	configs::{
		evm::{EvmChainId, GasScale, DEV_CHAIN_ID},
		TreasuryAccount, FEE_ANCHOR, MINER_FEE_SHARE,
	},
	genesis_config_presets as presets, AccountId, Balances, Executive, Revive, Runtime,
	RuntimeCall, RuntimeEvent, System, UncheckedExtrinsic, UNIT,
};
use pallet_revive::{
	evm::{Account, GenericTransaction},
	AccountId32Mapper, AddressMapper, Code, ExecConfig, StorageDeposit, TransactionLimits, H160,
	H256, U256,
};
use sp_consensus_pow::POW_ENGINE_ID;
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	generic,
	transaction_validity::{InvalidTransaction, TransactionSource, TransactionValidityError},
	DigestItem, DispatchError, TransactionOutcome,
};

/// The dev chain at block one, with Alith and the other dev EVM keys funded.
fn dev_ext() -> TestExternalities {
	let mut ext = preset_ext(sp_genesis_builder::DEV_RUNTIME_PRESET);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

fn baltathar() -> H160 {
	H160(hex_literal::hex!("3cd0a705a2dc65e5b1e1205896baa2be8a07c6e0"))
}

fn native_transfer(from: &Account, to: H160, value: U256) -> GenericTransaction {
	GenericTransaction {
		from: Some(from.address()),
		to: Some(to),
		value: Some(value),
		chain_id: Some(U256::from(EvmChainId::get())),
		nonce: Some(System::account_nonce(from.substrate_account()).into()),
		gas_price: Some(Revive::evm_base_fee()),
		..Default::default()
	}
}

fn dry_run(tx: GenericTransaction) -> U256 {
	with_transaction(|| {
		TransactionOutcome::Rollback(Ok::<_, DispatchError>(Revive::dry_run_eth_transact(
			tx, None, true, None,
		)))
	})
	.expect("the dry run rolls back")
	.expect("the dry run succeeds")
	.eth_gas
}

/// `tx` signed by `from`, wrapped the way an RPC node submits it.
fn eth_transact(from: &Account, mut tx: GenericTransaction) -> UncheckedExtrinsic {
	tx.gas = Some(dry_run(tx.clone()));
	let signed = from.sign_transaction(tx.try_into_unsigned().expect("a complete transaction"));
	let call =
		RuntimeCall::Revive(pallet_revive::Call::eth_transact { payload: signed.signed_payload() });
	generic::UncheckedExtrinsic::new_bare(call).into()
}

fn limits() -> TransactionLimits<Runtime> {
	TransactionLimits::WeightAndDeposit {
		weight_limit: Weight::from_parts(500_000_000_000, 5 * 1024 * 1024),
		deposit_limit: 10 * UNIT,
	}
}

#[test]
fn a_contract_deploys_under_the_one_plank_existential_deposit() {
	dev_ext().execute_with(|| {
		// Init code that deploys a single STOP byte.
		let init_code = hex_literal::hex!("6001600c60003960016000f300").to_vec();
		let deployer = Sr25519Keyring::Alice.to_account_id();

		let result = Revive::bare_instantiate(
			frame_system::RawOrigin::Signed(deployer).into(),
			U256::zero(),
			limits(),
			Code::Upload(init_code),
			Vec::new(),
			None,
			&ExecConfig::new_substrate_tx(),
		)
		.result
		.expect("the contract deploys");

		assert_eq!(Revive::code(&result.addr), vec![0x00]);
		assert!(System::account_exists(&AccountId32Mapper::<Runtime>::to_account_id(&result.addr)));
	});
}

#[test]
fn an_eth_transfer_passes_the_pool_and_lands_in_a_block() {
	dev_ext().execute_with(|| {
		let alith = Account::default();
		let received = Revive::evm_balance(&baltathar());
		let xt = eth_transact(&alith, native_transfer(&alith, baltathar(), U256::from(UNIT)));

		assert!(
			Executive::validate_transaction(TransactionSource::External, xt.clone(), System::parent_hash())
				.is_ok()
		);
		Executive::apply_extrinsic(xt)
			.expect("the block accepts the transfer")
			.expect("the transfer dispatches");

		assert_eq!(Revive::evm_balance(&baltathar()), received + U256::from(UNIT));
		assert_eq!(System::account_nonce(alith.substrate_account()), 1);
	});
}

/// Wallets and contracts written for Ethereum hard code 21,000 gas for a plain
/// transfer, so the gas the chain reports for one must not exceed it.
#[test]
fn a_native_transfer_reports_no_more_than_21000_gas() {
	dev_ext().execute_with(|| {
		let alith = Account::default();

		let gas = dry_run(native_transfer(&alith, baltathar(), U256::from(UNIT)));

		assert!(gas <= U256::from(21_000), "a native transfer reports {gas} gas");
	});
}

#[test]
fn the_gas_price_at_the_anchor_is_the_anchor_times_the_gas_scale() {
	dev_ext().execute_with(|| {
		assert_eq!(
			Revive::evm_base_fee(),
			U256::from(FEE_ANCHOR) * U256::from(GasScale::get()),
		);
	});
}

/// What an Ethereum transaction pays, the fee and the gas rounding left over,
/// splits like a substrate fee and none of it burns.
#[test]
fn eth_fees_split_between_the_author_and_the_treasury() {
	dev_ext().execute_with(|| {
		let author = Sr25519Keyring::Eve.to_account_id();
		System::deposit_log(DigestItem::PreRuntime(POW_ENGINE_ID, author.encode()));
		let alith = Account::default();
		let alith_before = Balances::free_balance(alith.substrate_account());
		let treasury_before = Balances::free_balance(TreasuryAccount::get());
		let author_before = Balances::free_balance(&author);
		let issuance = Balances::total_issuance();

		Executive::apply_extrinsic(eth_transact(
			&alith,
			native_transfer(&alith, baltathar(), U256::from(UNIT)),
		))
		.expect("the block accepts the transfer")
		.expect("the transfer dispatches");

		let fee = System::events()
			.into_iter()
			.find_map(|record| match record.event {
				RuntimeEvent::TransactionPayment(
					pallet_transaction_payment::Event::TransactionFeePaid { actual_fee, .. },
				) => Some(actual_fee),
				_ => None,
			})
			.expect("the transfer pays a fee");
		let paid = alith_before - Balances::free_balance(alith.substrate_account()) - UNIT;
		let kept = MINER_FEE_SHARE * fee + MINER_FEE_SHARE * (paid - fee);
		assert!(fee > 0 && paid >= fee);
		assert_eq!(Balances::free_balance(&author) - author_before, kept);
		assert_eq!(Balances::free_balance(TreasuryAccount::get()) - treasury_before, paid - kept);
		assert_eq!(Balances::total_issuance(), issuance, "no part of the fee burns");
	});
}

/// Contract storage costs what the same operation cost under Frontier at 1000
/// gwei. A new slot holds at least an SSTORE to a fresh slot and at most that
/// plus the cold slot surcharge.
#[test]
fn a_new_storage_slot_holds_what_an_sstore_cost_under_frontier() {
	const SSTORE_SET_GAS: u128 = 20_000;
	const COLD_SLOT_GAS: u128 = 2_100;
	dev_ext().execute_with(|| {
		// Init code that deploys `60003560005500`, which stores its first
		// calldata word in slot 0.
		let init_code = hex_literal::hex!("6007600c60003960076000f360003560005500").to_vec();
		let alice = || frame_system::RawOrigin::Signed(Sr25519Keyring::Alice.to_account_id()).into();
		let contract = Revive::bare_instantiate(
			alice(),
			U256::zero(),
			limits(),
			Code::Upload(init_code),
			Vec::new(),
			None,
			&ExecConfig::new_substrate_tx(),
		)
		.result
		.expect("the contract deploys")
		.addr;

		let write = Revive::bare_call(
			alice(),
			contract,
			U256::zero(),
			limits(),
			H256::from_low_u64_be(1).0.to_vec(),
			&ExecConfig::new_substrate_tx(),
		);

		assert_eq!(write.result.map(|output| output.did_revert()), Ok(false));
		let StorageDeposit::Charge(held) = write.storage_deposit else {
			panic!("a new slot holds a deposit");
		};
		let frontier_gas_price = 1_000 * (UNIT / 1_000_000_000);
		assert!(
			(SSTORE_SET_GAS * frontier_gas_price..=(SSTORE_SET_GAS + COLD_SLOT_GAS) * frontier_gas_price)
				.contains(&held),
			"a new slot holds {held}",
		);
	});
}

/// An Ethereum key pins its account with its first transaction, so the account
/// outlives being emptied and that transaction cannot replay once the address
/// is funded again.
#[test]
fn a_drained_eth_signer_keeps_its_nonce_so_its_transactions_cannot_replay() {
	dev_ext().execute_with(|| {
		let alith = Account::default();
		let who = alith.substrate_account();

		let pay = eth_transact(&alith, native_transfer(&alith, baltathar(), U256::from(UNIT)));
		Executive::apply_extrinsic(pay.clone()).expect("valid").expect("dispatches");

		let left = Balances::free_balance(&who);
		<Balances as Mutate<AccountId>>::burn_from(
			&who,
			left,
			Preservation::Expendable,
			Precision::Exact,
			Fortitude::Force,
		)
		.expect("the account empties");
		assert!(System::account_exists(&who), "the emptied signer stays in state");
		assert_eq!(System::account_nonce(&who), 1);

		<Balances as Mutate<AccountId>>::set_balance(&who, 10 * UNIT);
		assert_eq!(
			Executive::apply_extrinsic(pay).map(|_| ()),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
		);
	});
}

#[test]
fn a_new_account_resolves_from_its_own_evm_address() {
	dev_ext().execute_with(|| {
		let newcomer: AccountId = Sr25519Keyring::Charlie.to_account_id();

		let address = AccountId32Mapper::<Runtime>::to_address(&newcomer);

		assert!(AccountId32Mapper::<Runtime>::is_mapped(&newcomer));
		assert_eq!(AccountId32Mapper::<Runtime>::to_account_id(&address), newcomer);
	});
}

/// Reaping leaves the mapping of an account that never signed in place, so
/// value sent to its H160 later revives that same account.
#[test]
fn a_reaped_account_still_receives_at_its_evm_address() {
	dev_ext().execute_with(|| {
		let dave: AccountId = Sr25519Keyring::Dave.to_account_id();
		let address = AccountId32Mapper::<Runtime>::to_address(&dave);
		<Balances as Mutate<AccountId>>::transfer(
			&dave,
			&Sr25519Keyring::Eve.to_account_id(),
			Balances::free_balance(&dave),
			Preservation::Expendable,
		)
		.expect("the drain goes through");
		assert!(!System::account_exists(&dave), "a drained account that never signed goes");

		let alith = Account::default();
		Executive::apply_extrinsic(eth_transact(&alith, native_transfer(&alith, address, U256::from(UNIT))))
			.expect("the block accepts the transfer")
			.expect("the transfer dispatches");

		assert!(System::account_exists(&dave), "the payment revives the account");
		assert_eq!(Revive::evm_balance(&address), U256::from(UNIT));
	});
}

#[test]
fn every_network_signs_over_its_own_chain_id() {
	for (preset, chain_id) in [
		(sp_genesis_builder::DEV_RUNTIME_PRESET, DEV_CHAIN_ID),
		(sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET, DEV_CHAIN_ID),
		(presets::INTEGRATION_RUNTIME_PRESET, DEV_CHAIN_ID),
		(presets::TESTNET_RUNTIME_PRESET, 320_261),
		(presets::MAINNET_RUNTIME_PRESET, 32_026),
	] {
		preset_ext(preset).execute_with(|| {
			assert_eq!(EvmChainId::get(), chain_id, "preset {preset}");
		});
	}
}

/// The chain id running networks already hold sits under the key the EVM layer
/// before pallet-revive wrote it to, and is read from there as is.
#[test]
fn the_stored_chain_id_is_read_where_it_already_lies() {
	TestExternalities::default().execute_with(|| {
		let key =
			[sp_io::hashing::twox_128(b"EVMChainId"), sp_io::hashing::twox_128(b"ChainId")].concat();
		unhashed::put_raw(&key, &32_026u64.encode());

		assert_eq!(EvmChainId::get(), 32_026);
	});
}
