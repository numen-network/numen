//! The native token's ERC20 at `0x…0802`, driven through the proxy contract
//! the way wallets and contracts reach it. It reads and moves the balance the
//! chain keeps and refunds value sent as a deposit. It accepts permits signed
//! for the live network, answers only through the proxy and reads the
//! allowances already on chain.

mod common;

use codec::Encode;
use common::preset_ext;
use frame_support::{
	storage::{unhashed, with_transaction},
	traits::{
		fungible::{Inspect, Mutate},
		Get,
	},
	weights::Weight,
	Blake2_128Concat, StorageHasher,
};
use numen_runtime::{
	balances_erc20::{proxy_code, Allowances, IBalancesErc20, Nonces, ADDRESS, PRECOMPILE},
	configs::evm::EvmChainId,
	genesis_config_presets as presets, AccountId, Balance, Balances, Block, BlockNumber, Executive,
	Nonce, Revive, Runtime, RuntimeCall, RuntimeEvent, RuntimeOrigin, System, Timestamp,
	UncheckedExtrinsic, UNIT,
};
use pallet_revive::{
	evm::{Account, GenericTransaction},
	pallet_revive_types::runtime_api::{
		TransactInputPayloadV1, TransactOutputPayloadV1, TransactVersionedInputPayload,
	},
	precompiles::alloy::{
		primitives::{Address, B256, U256 as EvmU256},
		sol_types::{Revert, SolCall, SolError, SolEvent},
	},
	AccountId32Mapper, AddressMapper, Code, ExecConfig, ExecReturnValue, TransactionLimits, H160,
	H256, U256,
};
use pallet_revive::runtime_decl_for_revive_api::ReviveApi;
use sp_core::Pair;
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{generic, DispatchError, TransactionOutcome};

/// Secret key of the permit signer, any valid secp256k1 scalar.
const OWNER_KEY: [u8; 32] = [0x42; 32];

fn dev_ext() -> TestExternalities {
	let mut ext = preset_ext(sp_genesis_builder::DEV_RUNTIME_PRESET);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

fn alice() -> AccountId {
	Sr25519Keyring::Alice.to_account_id()
}

fn bob() -> AccountId {
	Sr25519Keyring::Bob.to_account_id()
}

/// A dev account with no role, so nothing locks its balance.
fn ferdie() -> AccountId {
	Sr25519Keyring::Ferdie.to_account_id()
}

fn address_of(account: &AccountId) -> H160 {
	AccountId32Mapper::<Runtime>::to_address(account)
}

fn alloy(address: H160) -> Address {
	Address::from(address.0)
}

fn limits() -> TransactionLimits<Runtime> {
	TransactionLimits::WeightAndDeposit {
		weight_limit: Weight::from_parts(500_000_000_000, 5 * 1024 * 1024),
		deposit_limit: 10 * UNIT,
	}
}

fn call(who: &AccountId, dest: H160, value: u128, data: Vec<u8>) -> ExecReturnValue {
	Revive::bare_call(
		RuntimeOrigin::signed(who.clone()),
		dest,
		U256::from(value),
		limits(),
		data,
		&ExecConfig::new_substrate_tx(),
	)
	.result
	.expect("the call executes")
}

fn token(who: &AccountId, data: Vec<u8>) -> ExecReturnValue {
	call(who, ADDRESS, 0, data)
}

fn succeeded(output: ExecReturnValue) -> Vec<u8> {
	assert!(!output.did_revert(), "reverted with {:?}", Revert::abi_decode(&output.data));
	output.data
}

fn revert_reason(output: ExecReturnValue) -> String {
	assert!(output.did_revert(), "the call succeeded");
	Revert::abi_decode(&output.data).expect("a revert reason").reason
}

fn balance_of(owner: H160) -> EvmU256 {
	let data = succeeded(token(&alice(), IBalancesErc20::balanceOfCall { owner: alloy(owner) }.abi_encode()));
	IBalancesErc20::balanceOfCall::abi_decode_returns(&data).expect("a balance")
}

fn transfer(to: H160, value: u128) -> Vec<u8> {
	IBalancesErc20::transferCall { to: alloy(to), value: EvmU256::from(value) }.abi_encode()
}

/// Logs the token emitted, as `(emitter, topics, data)`.
fn logs() -> Vec<(H160, Vec<B256>, Vec<u8>)> {
	System::events()
		.into_iter()
		.filter_map(|record| match record.event {
			RuntimeEvent::Revive(pallet_revive::Event::ContractEmitted { contract, data, topics }) =>
				Some((contract, topics.into_iter().map(|topic| B256::from(topic.0)).collect(), data)),
			_ => None,
		})
		.collect()
}

fn single_log<E: SolEvent>() -> E {
	let logs = logs();
	assert_eq!(logs.len(), 1, "one log, got {logs:?}");
	let (emitter, topics, data) = &logs[0];
	assert_eq!(*emitter, ADDRESS, "the log comes from the token address");
	E::decode_raw_log(topics.iter().copied(), data).expect("the expected event")
}

/// A contract that runs `forward` on its calldata and passes the result on,
/// success or revert.
fn deploy_forwarder(forward: &[u8]) -> H160 {
	let mut runtime = vec![0x36, 0x60, 0, 0x60, 0, 0x37];
	runtime.extend(forward);
	runtime.extend([0x3d, 0x60, 0, 0x60, 0, 0x3e]);
	let success = (runtime.len() + 7) as u8;
	runtime.extend([0x60, success, 0x57, 0x3d, 0x60, 0, 0xfd, 0x5b, 0x3d, 0x60, 0, 0xf3]);
	let len = runtime.len() as u8;
	let mut init = vec![0x60, len, 0x60, 12, 0x60, 0, 0x39, 0x60, len, 0x60, 0, 0xf3];
	init.extend(runtime);

	Revive::bare_instantiate(
		RuntimeOrigin::signed(alice()),
		U256::zero(),
		limits(),
		Code::Upload(init.clone()),
		Vec::new(),
		Some(keccak(&init)),
		&ExecConfig::new_substrate_tx(),
	)
	.result
	.expect("the forwarder deploys")
	.addr
}

/// Code that calls `target` with the calldata as input through the call opcode
/// `kind`, sending no value when that is `CALL`.
fn forward(kind: u8, target: H160) -> Vec<u8> {
	let mut code = vec![0x60, 0, 0x60, 0, 0x36, 0x60, 0];
	if kind == 0xf1 {
		code.extend([0x60, 0]);
	}
	code.push(0x73);
	code.extend(target.as_bytes());
	code.extend([0x5a, kind]);
	code
}

const CALL: u8 = 0xf1;
const DELEGATECALL: u8 = 0xf4;
const STATICCALL: u8 = 0xfa;

#[test]
fn the_token_reports_its_metadata() {
	dev_ext().execute_with(|| {
		let name = succeeded(token(&alice(), IBalancesErc20::nameCall {}.abi_encode()));
		let symbol = succeeded(token(&alice(), IBalancesErc20::symbolCall {}.abi_encode()));
		let decimals = succeeded(token(&alice(), IBalancesErc20::decimalsCall {}.abi_encode()));

		assert_eq!(IBalancesErc20::nameCall::abi_decode_returns(&name).unwrap(), "Numen");
		assert_eq!(IBalancesErc20::symbolCall::abi_decode_returns(&symbol).unwrap(), "NUMN");
		assert_eq!(IBalancesErc20::decimalsCall::abi_decode_returns(&decimals).unwrap(), 18);
	});
}

#[test]
fn total_supply_is_the_total_issuance() {
	dev_ext().execute_with(|| {
		let data = succeeded(token(&alice(), IBalancesErc20::totalSupplyCall {}.abi_encode()));

		assert_eq!(
			IBalancesErc20::totalSupplyCall::abi_decode_returns(&data).unwrap(),
			EvmU256::from(Balances::total_issuance()),
		);
	});
}

/// `totalSupply()` through the runtime API behind `eth_call`, with `gas` as
/// the caller gives it.
fn total_supply_in_a_dry_run(gas: Option<U256>) -> EvmU256 {
	let tx = GenericTransaction {
		from: Some(Account::default().address()),
		to: Some(ADDRESS),
		input: IBalancesErc20::totalSupplyCall {}.abi_encode().into(),
		gas,
		..Default::default()
	};
	let input = TransactVersionedInputPayload::V1(TransactInputPayloadV1 {
		tx: tx.into(),
		timestamp_override: None,
		perform_balance_checks: true,
		state_overrides: None,
	});
	let output = <Runtime as ReviveApi<Block, AccountId, Balance, Nonce, BlockNumber, u64>>::eth_transact_versioned(input)
		.expect("the dry run succeeds");
	let data = TransactOutputPayloadV1::try_from(output).expect("a v1 output").transact_info.data;
	IBalancesErc20::totalSupplyCall::abi_decode_returns(&data).expect("a supply")
}

#[test]
fn total_supply_in_a_dry_run_is_the_total_issuance() {
	for gas in [None, Some(U256::from(21_000)), Some(U256::from(1_000_000))] {
		dev_ext().execute_with(|| {
			let issuance = EvmU256::from(Balances::total_issuance());

			assert_eq!(total_supply_in_a_dry_run(gas), issuance, "gas {gas:?}");
		});
	}
}

/// A contract that stores what `totalSupply()` returns in slot zero.
fn deploy_supply_recorder() -> H160 {
	let mut runtime = vec![0x63];
	runtime.extend(IBalancesErc20::totalSupplyCall::SELECTOR);
	runtime.extend([0x60, 0xe0, 0x1b, 0x60, 0, 0x52, 0x60, 0x20, 0x60, 0, 0x60, 4, 0x60, 0, 0x73]);
	runtime.extend(ADDRESS.as_bytes());
	runtime.extend([0x5a, STATICCALL, 0x50, 0x60, 0, 0x51, 0x60, 0, 0x55, 0x00]);
	let len = runtime.len() as u8;
	let mut init = vec![0x60, len, 0x60, 12, 0x60, 0, 0x39, 0x60, len, 0x60, 0, 0xf3];
	init.extend(runtime);

	Revive::bare_instantiate(
		RuntimeOrigin::signed(alice()),
		U256::zero(),
		limits(),
		Code::Upload(init.clone()),
		Vec::new(),
		Some(keccak(&init)),
		&ExecConfig::new_substrate_tx(),
	)
	.result
	.expect("the recorder deploys")
	.addr
}

/// A call to `to` signed by `from`, wrapped the way an RPC node submits it.
fn eth_transact(from: &Account, to: H160) -> UncheckedExtrinsic {
	let mut tx = GenericTransaction {
		from: Some(from.address()),
		to: Some(to),
		chain_id: Some(U256::from(EvmChainId::get())),
		nonce: Some(System::account_nonce(from.substrate_account()).into()),
		gas_price: Some(Revive::evm_base_fee()),
		..Default::default()
	};
	let estimate = with_transaction(|| {
		TransactionOutcome::Rollback(Ok::<_, DispatchError>(Revive::dry_run_eth_transact(
			tx.clone(),
			None,
			true,
			None,
		)))
	})
	.expect("the dry run rolls back")
	.expect("the dry run succeeds");
	tx.gas = Some(estimate.eth_gas);
	let signed = from.sign_transaction(tx.try_into_unsigned().expect("a complete transaction"));
	let call =
		RuntimeCall::Revive(pallet_revive::Call::eth_transact { payload: signed.signed_payload() });
	generic::UncheckedExtrinsic::new_bare(call).into()
}

#[test]
fn total_supply_in_a_transaction_is_the_total_issuance() {
	dev_ext().execute_with(|| {
		let recorder = deploy_supply_recorder();
		let issuance = Balances::total_issuance();

		Executive::apply_extrinsic(eth_transact(&Account::default(), recorder))
			.expect("the block accepts the call")
			.expect("the call dispatches");

		let recorded = Revive::get_storage(recorder, [0; 32])
			.expect("the recorder is a contract")
			.expect("the recorder stored the supply");
		assert_eq!(U256::from_big_endian(&recorded), U256::from(issuance));
	});
}

/// What `balanceOf` shows is what the account can move, the same figure a
/// wallet shows for the native balance.
#[test]
fn balance_of_is_the_spendable_balance() {
	dev_ext().execute_with(|| {
		let ferdie_address = address_of(&ferdie());

		assert_eq!(balance_of(ferdie_address), EvmU256::from(Balances::free_balance(ferdie()) - 1));
		assert_eq!(
			balance_of(ferdie_address),
			EvmU256::from_be_bytes(Revive::evm_balance(&ferdie_address).to_big_endian()),
		);
		assert_eq!(balance_of(H160::repeat_byte(0x77)), EvmU256::ZERO);
	});
}

#[test]
fn a_transfer_moves_funds_and_logs_from_the_token_address() {
	dev_ext().execute_with(|| {
		let alice_before = Balances::free_balance(alice());
		let bob_before = Balances::free_balance(bob());

		let data = succeeded(token(&alice(), transfer(address_of(&bob()), 5 * UNIT)));

		assert!(IBalancesErc20::transferCall::abi_decode_returns(&data).unwrap());
		assert_eq!(Balances::free_balance(alice()), alice_before - 5 * UNIT);
		assert_eq!(Balances::free_balance(bob()), bob_before + 5 * UNIT);
		let event = single_log::<IBalancesErc20::Transfer>();
		assert_eq!(event.from, alloy(address_of(&alice())));
		assert_eq!(event.to, alloy(address_of(&bob())));
		assert_eq!(event.value, EvmU256::from(5 * UNIT));
	});
}

/// `transfer(to, balanceOf(self))` empties what an Ethereum account shows yet
/// keeps the account, so its nonce survives.
#[test]
fn sending_the_whole_balance_leaves_the_existential_deposit() {
	dev_ext().execute_with(|| {
		let everything = balance_of(address_of(&ferdie()));

		succeeded(token(
			&ferdie(),
			IBalancesErc20::transferCall { to: alloy(address_of(&bob())), value: everything }
				.abi_encode(),
		));

		assert_eq!(Balances::free_balance(ferdie()), 1);
		assert_eq!(balance_of(address_of(&ferdie())), EvmU256::ZERO);
	});
}

/// An address that holds nothing yet receives the amount whole, as it does
/// from a plain value transfer.
#[test]
fn a_transfer_to_a_new_account_arrives_whole() {
	dev_ext().execute_with(|| {
		let newcomer = H160::repeat_byte(0x66);
		let issuance = Balances::total_issuance();
		let active = Balances::active_issuance();

		succeeded(token(&alice(), transfer(newcomer, 2 * UNIT)));

		assert_eq!(balance_of(newcomer), EvmU256::from(2 * UNIT));
		assert_eq!(Balances::total_issuance(), issuance + 1);
		assert_eq!(Balances::active_issuance(), active);
	});
}

#[test]
fn a_zero_transfer_succeeds_and_moves_nothing() {
	dev_ext().execute_with(|| {
		let alice_before = Balances::free_balance(alice());

		succeeded(token(&alice(), transfer(address_of(&bob()), 0)));

		assert_eq!(Balances::free_balance(alice()), alice_before);
		assert_eq!(single_log::<IBalancesErc20::Transfer>().value, EvmU256::ZERO);
	});
}

/// A transfer to oneself moves nothing, yet still has to be covered.
#[test]
fn a_self_transfer_within_the_balance_succeeds() {
	dev_ext().execute_with(|| {
		let alice_before = Balances::free_balance(alice());

		succeeded(token(&alice(), transfer(address_of(&alice()), UNIT)));

		assert_eq!(Balances::free_balance(alice()), alice_before);
		assert_eq!(single_log::<IBalancesErc20::Transfer>().value, EvmU256::from(UNIT));
	});
}

#[test]
fn a_self_transfer_beyond_the_balance_reverts() {
	dev_ext().execute_with(|| {
		let alice_before = Balances::free_balance(alice());

		let reason = revert_reason(token(&alice(), transfer(address_of(&alice()), alice_before)));

		assert_eq!(reason, "ERC20: transfer failed");
		assert!(logs().is_empty());
	});
}

#[test]
fn a_transfer_beyond_the_balance_reverts() {
	dev_ext().execute_with(|| {
		let alice_before = Balances::free_balance(alice());

		let reason = revert_reason(token(&alice(), transfer(address_of(&bob()), alice_before)));

		assert_eq!(reason, "ERC20: transfer failed");
		assert_eq!(Balances::free_balance(alice()), alice_before);
		assert!(logs().is_empty());
	});
}

#[test]
fn an_amount_past_the_balance_type_reverts() {
	dev_ext().execute_with(|| {
		let data =
			IBalancesErc20::transferCall { to: alloy(address_of(&bob())), value: EvmU256::MAX }
				.abi_encode();

		assert_eq!(revert_reason(token(&alice(), data)), "ERC20: amount overflow");
	});
}

#[test]
fn transfer_from_spends_the_allowance() {
	dev_ext().execute_with(|| {
		let charlie = Sr25519Keyring::Charlie.to_account_id();
		let approve = IBalancesErc20::approveCall {
			spender: alloy(address_of(&bob())),
			value: EvmU256::from(10 * UNIT),
		};
		succeeded(token(&alice(), approve.abi_encode()));
		let charlie_before = Balances::free_balance(&charlie);

		let pull = IBalancesErc20::transferFromCall {
			from: alloy(address_of(&alice())),
			to: alloy(address_of(&charlie)),
			value: EvmU256::from(4 * UNIT),
		};
		succeeded(token(&bob(), pull.abi_encode()));

		assert_eq!(Balances::free_balance(&charlie), charlie_before + 4 * UNIT);
		let left = succeeded(token(
			&bob(),
			IBalancesErc20::allowanceCall {
				owner: alloy(address_of(&alice())),
				spender: alloy(address_of(&bob())),
			}
			.abi_encode(),
		));
		assert_eq!(
			IBalancesErc20::allowanceCall::abi_decode_returns(&left).unwrap(),
			EvmU256::from(6 * UNIT),
		);
	});
}

#[test]
fn transfer_from_beyond_the_allowance_reverts() {
	dev_ext().execute_with(|| {
		let pull = IBalancesErc20::transferFromCall {
			from: alloy(address_of(&alice())),
			to: alloy(address_of(&bob())),
			value: EvmU256::from(1),
		};

		assert_eq!(revert_reason(token(&bob(), pull.abi_encode())), "ERC20: insufficient allowance");
	});
}

#[test]
fn transfer_from_oneself_needs_no_allowance() {
	dev_ext().execute_with(|| {
		let bob_before = Balances::free_balance(bob());
		let pull = IBalancesErc20::transferFromCall {
			from: alloy(address_of(&alice())),
			to: alloy(address_of(&bob())),
			value: EvmU256::from(UNIT),
		};

		succeeded(token(&alice(), pull.abi_encode()));

		assert_eq!(Balances::free_balance(bob()), bob_before + UNIT);
	});
}

#[test]
fn withdraw_pays_a_substrate_account() {
	dev_ext().execute_with(|| {
		let dest = Sr25519Keyring::Dave.to_account_id();
		let dest_before = Balances::free_balance(&dest);
		let raw: [u8; 32] = dest.clone().into();

		let data = IBalancesErc20::withdrawCall { dest: raw.into(), value: EvmU256::from(3 * UNIT) };
		succeeded(token(&alice(), data.abi_encode()));

		assert_eq!(Balances::free_balance(&dest), dest_before + 3 * UNIT);
		let event = single_log::<IBalancesErc20::Withdrawal>();
		assert_eq!(event.src, alloy(address_of(&alice())));
		assert_eq!(event.dest, B256::from(raw));
	});
}

/// Value sent with `deposit()`, with no calldata or with a selector the token
/// does not know, comes straight back and leaves a Deposit log.
#[test]
fn value_sent_as_a_deposit_comes_back() {
	for data in [IBalancesErc20::depositCall {}.abi_encode(), Vec::new(), vec![0xde, 0xad, 0xbe, 0xef]] {
		dev_ext().execute_with(|| {
			let alice_before = Balances::free_balance(alice());
			let token_before = Balances::free_balance(AccountId32Mapper::<Runtime>::to_account_id(&ADDRESS));

			succeeded(call(&alice(), ADDRESS, 7 * UNIT, data.clone()));

			assert_eq!(Balances::free_balance(alice()), alice_before, "calldata {data:x?}");
			assert_eq!(
				Balances::free_balance(AccountId32Mapper::<Runtime>::to_account_id(&ADDRESS)),
				token_before,
			);
			let event = single_log::<IBalancesErc20::Deposit>();
			assert_eq!(event.dst, alloy(address_of(&alice())));
			assert_eq!(event.wad, EvmU256::from(7 * UNIT));
		});
	}
}

#[test]
fn a_deposit_without_value_reverts() {
	for data in [IBalancesErc20::depositCall {}.abi_encode(), Vec::new(), vec![0xde, 0xad, 0xbe, 0xef]] {
		dev_ext().execute_with(|| {
			assert_eq!(revert_reason(token(&alice(), data)), "ERC20: cannot deposit zero");
		});
	}
}

#[test]
fn value_sent_to_any_other_function_reverts() {
	dev_ext().execute_with(|| {
		let alice_before = Balances::free_balance(alice());

		let reason = revert_reason(call(&alice(), ADDRESS, UNIT, transfer(address_of(&bob()), 1)));

		assert_eq!(reason, "Function is not payable");
		assert_eq!(Balances::free_balance(alice()), alice_before);
	});
}

/// The separator mainnet already hands out, so permits signed against it stay
/// valid.
#[test]
fn mainnet_keeps_its_domain_separator() {
	preset_ext(presets::MAINNET_RUNTIME_PRESET).execute_with(|| {
		<Balances as Mutate<AccountId>>::mint_into(&alice(), UNIT).unwrap();

		let data = succeeded(token(&alice(), IBalancesErc20::DOMAIN_SEPARATORCall {}.abi_encode()));

		assert_eq!(
			IBalancesErc20::DOMAIN_SEPARATORCall::abi_decode_returns(&data).unwrap(),
			B256::from(hex_literal::hex!(
				"0ba013f2ed6677bc5551764f058622c29d6ad4781ae50567190829c8adf40fad"
			)),
		);
	});
}

fn keccak(data: &[u8]) -> [u8; 32] {
	sp_io::hashing::keccak_256(data)
}

/// `permit` call data for `value` at `nonce`, signed by [`OWNER_KEY`].
fn signed_permit(spender: H160, value: u128, nonce: u64, deadline: u64) -> IBalancesErc20::permitCall {
	let owner = Account::from_secret_key(OWNER_KEY).address();
	let separator = succeeded(token(&alice(), IBalancesErc20::DOMAIN_SEPARATORCall {}.abi_encode()));
	let struct_hash = keccak(
		&[
			keccak(b"Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)")
				.to_vec(),
			H256::from(owner).0.to_vec(),
			H256::from(spender).0.to_vec(),
			U256::from(value).to_big_endian().to_vec(),
			U256::from(nonce).to_big_endian().to_vec(),
			U256::from(deadline).to_big_endian().to_vec(),
		]
		.concat(),
	);
	let digest = keccak(&[b"\x19\x01".to_vec(), separator, struct_hash.to_vec()].concat());
	let signature = sp_core::ecdsa::Pair::from_seed(&OWNER_KEY).sign_prehashed(&digest);
	let signature: &[u8] = signature.as_ref();

	IBalancesErc20::permitCall {
		owner: alloy(owner),
		spender: alloy(spender),
		value: EvmU256::from(value),
		deadline: EvmU256::from(deadline),
		v: signature[64] + 27,
		r: B256::from_slice(&signature[..32]),
		s: B256::from_slice(&signature[32..64]),
	}
}

#[test]
fn a_permit_sets_the_allowance_once() {
	dev_ext().execute_with(|| {
		let owner = Account::from_secret_key(OWNER_KEY).address();
		let spender = address_of(&bob());
		let permit = signed_permit(spender, 9 * UNIT, 0, u64::MAX);

		succeeded(token(&alice(), permit.abi_encode()));

		assert_eq!(Allowances::get(owner, spender), U256::from(9 * UNIT));
		assert_eq!(Nonces::get(owner), U256::one());
		assert_eq!(single_log::<IBalancesErc20::Approval>().value, EvmU256::from(9 * UNIT));
		assert_eq!(revert_reason(token(&alice(), permit.abi_encode())), "Invalid permit");
	});
}

#[test]
fn an_expired_permit_reverts() {
	dev_ext().execute_with(|| {
		Timestamp::set_timestamp(2_000_000);
		let permit = signed_permit(address_of(&bob()), UNIT, 0, 1_999);

		assert_eq!(revert_reason(token(&alice(), permit.abi_encode())), "Permit expired");
	});
}

#[test]
fn a_permit_for_someone_else_reverts() {
	dev_ext().execute_with(|| {
		let mut permit = signed_permit(address_of(&bob()), UNIT, 0, u64::MAX);
		permit.owner = alloy(address_of(&alice()));

		assert_eq!(revert_reason(token(&alice(), permit.abi_encode())), "Invalid permit");
		assert_eq!(Allowances::get(address_of(&alice()), address_of(&bob())), U256::zero());
	});
}

/// The precompile acts for whoever called the frame it runs in. Reached any
/// way but through the proxy, that is not the token holder, so it refuses.
#[test]
fn the_precompile_answers_only_through_the_proxy() {
	dev_ext().execute_with(|| {
		let theft = transfer(address_of(&bob()), UNIT);
		let borrow_token = deploy_forwarder(&forward(DELEGATECALL, ADDRESS));
		let borrow_precompile = deploy_forwarder(&forward(DELEGATECALL, PRECOMPILE));
		let alice_before = Balances::free_balance(alice());

		for (dest, route) in [
			(PRECOMPILE, "a direct call"),
			(borrow_token, "a delegatecall to the token"),
			(borrow_precompile, "a delegatecall to the precompile"),
		] {
			assert_eq!(
				revert_reason(call(&alice(), dest, 0, theft.clone())),
				"ERC20: call the token at 0x0000000000000000000000000000000000000802",
				"{route}",
			);
		}
		assert_eq!(Balances::free_balance(alice()), alice_before);
	});
}

#[test]
fn a_contract_spends_its_own_balance_through_a_plain_call() {
	dev_ext().execute_with(|| {
		let contract = deploy_forwarder(&forward(CALL, ADDRESS));
		let contract_account = AccountId32Mapper::<Runtime>::to_account_id(&contract);
		<Balances as Mutate<AccountId>>::mint_into(&contract_account, 10 * UNIT).unwrap();
		let bob_before = Balances::free_balance(bob());
		System::reset_events();

		succeeded(call(&alice(), contract, 0, transfer(address_of(&bob()), 4 * UNIT)));

		assert_eq!(Balances::free_balance(bob()), bob_before + 4 * UNIT);
		assert_eq!(single_log::<IBalancesErc20::Transfer>().from, alloy(contract));
	});
}

#[test]
fn a_static_call_moves_nothing() {
	dev_ext().execute_with(|| {
		let viewer = deploy_forwarder(&forward(STATICCALL, ADDRESS));
		let viewer_account = AccountId32Mapper::<Runtime>::to_account_id(&viewer);
		<Balances as Mutate<AccountId>>::mint_into(&viewer_account, 10 * UNIT).unwrap();
		let viewer_before = Balances::free_balance(&viewer_account);

		let reason = revert_reason(call(&alice(), viewer, 0, transfer(address_of(&bob()), UNIT)));

		assert_eq!(reason, "Can't call non-static function in static context");
		assert_eq!(Balances::free_balance(&viewer_account), viewer_before);
	});
}

#[test]
fn a_root_call_has_no_account_to_act_for() {
	dev_ext().execute_with(|| {
		let output = Revive::bare_call(
			RuntimeOrigin::root(),
			ADDRESS,
			U256::zero(),
			limits(),
			transfer(address_of(&bob()), UNIT),
			&ExecConfig::new_substrate_tx(),
		)
		.result
		.expect("the call executes");

		assert_eq!(revert_reason(output), "ERC20: no account to act for");
	});
}

/// Mainnet holds allowances the Frontier precompile wrote. They sit under the
/// same storage key and count as they are.
#[test]
fn allowances_already_on_chain_are_honoured() {
	dev_ext().execute_with(|| {
		let owner = address_of(&alice());
		let spender = address_of(&bob());
		let key = [
			sp_io::hashing::twox_128(b"EvmBalancesErc20").to_vec(),
			sp_io::hashing::twox_128(b"Allowances").to_vec(),
			Blake2_128Concat::hash(&owner.encode()),
			Blake2_128Concat::hash(&spender.encode()),
		]
		.concat();
		unhashed::put_raw(&key, &U256::from(3 * UNIT).encode());

		let data = succeeded(token(
			&bob(),
			IBalancesErc20::allowanceCall { owner: alloy(owner), spender: alloy(spender) }.abi_encode(),
		));

		assert_eq!(
			IBalancesErc20::allowanceCall::abi_decode_returns(&data).unwrap(),
			EvmU256::from(3 * UNIT),
		);
	});
}

#[test]
fn every_network_starts_with_the_proxy_in_place() {
	for preset in [
		sp_genesis_builder::DEV_RUNTIME_PRESET,
		presets::TESTNET_RUNTIME_PRESET,
		presets::MAINNET_RUNTIME_PRESET,
	] {
		preset_ext(preset).execute_with(|| {
			assert_eq!(Revive::code(&ADDRESS), proxy_code(), "preset {preset}");
		});
	}
}

/// Running networks keep the proxy code they were given. A different proxy
/// needs a migration that rewrites the code at the token address. This hash
/// moves with it.
#[test]
fn the_proxy_code_is_pinned() {
	assert_eq!(
		keccak(&proxy_code()),
		hex_literal::hex!("78ac627290058598a425d1348dc783ea0b7845b44c5d541ee0ab0c9d2e876e67"),
	);
}
