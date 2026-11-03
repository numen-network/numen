//! What Frontier kept at an H160's derived account, claimed over to the account
//! pallet-revive resolves the H160 to. A claim travels as an authorized
//! transaction that pays its fee out of what it moves and leaves the H160 with
//! the nonce it had. An Ethereum signer nobody claimed for still cannot replay
//! what Frontier ran.

mod common;

use common::preset_ext;
use frame_support::traits::fungible::{Inspect, Mutate};
use frame_system::offchain::CreateAuthorizedTransaction;
use numen_runtime::{
	configs::{
		evm::{ClaimFee, EvmChainId},
		TreasuryAccount,
	},
	AccountId, Balances, Executive, Revive, Runtime, RuntimeCall, System, UncheckedExtrinsic, UNIT,
};
use pallet_revive::{
	evm::{Account, GenericTransaction},
	AccountId32Mapper, AddressMapper, H160, U256,
};
use sp_io::TestExternalities;
use sp_runtime::{
	generic,
	traits::Get,
	transaction_validity::{InvalidTransaction, TransactionSource, TransactionValidityError},
};

type EvmClaims = pallet_evm_claims::Pallet<Runtime>;

fn dev_ext() -> TestExternalities {
	let mut ext = preset_ext(sp_genesis_builder::DEV_RUNTIME_PRESET);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// An H160 whose funds still sit where Frontier put them.
fn frontier_user() -> Account {
	Account::from_secret_key([0x31; 32])
}

/// `balance` and `nonce` at the account Frontier derived for `address`.
fn frontier_account(address: H160, balance: u128, nonce: u32) {
	let legacy = EvmClaims::legacy_account(&address);
	System::inc_providers(&legacy);
	Balances::set_balance(&legacy, balance);
	frame_system::Account::<Runtime>::mutate(&legacy, |info| info.nonce = nonce);
}

fn claim(address: H160) -> UncheckedExtrinsic {
	<Runtime as CreateAuthorizedTransaction<RuntimeCall>>::create_authorized_transaction(
		RuntimeCall::EvmClaims(pallet_evm_claims::Call::claim { address }),
	)
}

/// A native transfer `from` signs at `nonce`, wrapped the way an RPC node
/// submits it.
fn transfer_at(from: &Account, nonce: u32) -> UncheckedExtrinsic {
	let tx = GenericTransaction {
		from: Some(from.address()),
		to: Some(H160::repeat_byte(0x55)),
		value: Some(U256::from(UNIT)),
		chain_id: Some(U256::from(EvmChainId::get())),
		nonce: Some(nonce.into()),
		gas_price: Some(Revive::evm_base_fee()),
		gas: Some(U256::from(100_000)),
		..Default::default()
	};
	let signed = from.sign_transaction(tx.try_into_unsigned().expect("a complete transaction"));
	let call =
		RuntimeCall::Revive(pallet_revive::Call::eth_transact { payload: signed.signed_payload() });
	generic::UncheckedExtrinsic::new_bare(call).into()
}

/// Nobody signs or pays for the claim up front. Its fee comes out of the
/// balance it moves and, in a block without an author, goes to the treasury.
#[test]
fn a_claim_passes_the_pool_and_pays_its_fee_from_what_it_moves() {
	dev_ext().execute_with(|| {
		let address = frontier_user().address();
		frontier_account(address, 1_234 * UNIT, 7);
		let issuance = Balances::total_issuance();
		let treasury = Balances::balance(&TreasuryAccount::get());
		let fee = ClaimFee::get();
		let xt = claim(address);

		assert!(
			Executive::validate_transaction(TransactionSource::External, xt.clone(), System::parent_hash())
				.is_ok()
		);
		Executive::apply_extrinsic(xt)
			.expect("the block accepts the claim")
			.expect("the claim dispatches");

		let account: AccountId = AccountId32Mapper::<Runtime>::to_account_id(&address);
		assert_eq!(account, EvmClaims::revive_account(&address));
		assert!(fee > 0);
		assert_eq!(Revive::evm_balance(&address), U256::from(1_234 * UNIT - fee));
		assert_eq!(System::account_nonce(&account), 7);
		assert_eq!(Balances::total_issuance(), issuance + 1, "only the existential deposit is new");
		assert_eq!(Balances::balance(&TreasuryAccount::get()), treasury + fee);
	});
}

#[test]
fn a_claim_with_nothing_to_move_is_turned_away() {
	dev_ext().execute_with(|| {
		let refused = Err(TransactionValidityError::Invalid(InvalidTransaction::Custom(
			pallet_evm_claims::NOTHING_TO_CLAIM,
		)));
		let xt = claim(frontier_user().address());

		assert_eq!(
			Executive::validate_transaction(TransactionSource::External, xt.clone(), System::parent_hash())
				.map(|_| ()),
			refused,
		);
		assert_eq!(Executive::apply_extrinsic(xt).map(|_| ()), refused);
	});
}

/// A balance worth the fee or less buys nobody a claim's block space.
#[test]
fn a_balance_worth_no_more_than_the_fee_is_turned_away() {
	dev_ext().execute_with(|| {
		let address = frontier_user().address();
		frontier_account(address, ClaimFee::get(), 0);
		let refused = Err(TransactionValidityError::Invalid(InvalidTransaction::Custom(
			pallet_evm_claims::NOTHING_TO_CLAIM,
		)));

		assert_eq!(
			Executive::validate_transaction(
				TransactionSource::External,
				claim(address),
				System::parent_hash(),
			)
			.map(|_| ()),
			refused,
		);
		assert_eq!(Executive::apply_extrinsic(claim(address)).map(|_| ()), refused);
	});
}

/// The signer's old account was never claimed. A transaction Frontier
/// already ran for it is still stale, and the signer picks up at its
/// Frontier nonce.
#[test]
fn an_unclaimed_signer_starts_at_its_frontier_nonce() {
	dev_ext().execute_with(|| {
		let alith = Account::default();
		frontier_account(alith.address(), 0, 3);

		assert_eq!(
			Executive::validate_transaction(
				TransactionSource::External,
				transfer_at(&alith, 2),
				System::parent_hash(),
			)
			.map(|_| ()),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
		);
		assert_eq!(
			Executive::apply_extrinsic(transfer_at(&alith, 0)).map(|_| ()),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
		);
		Executive::apply_extrinsic(transfer_at(&alith, 3))
			.expect("the block accepts the next transaction")
			.expect("the transfer dispatches");
		assert_eq!(System::account_nonce(alith.substrate_account()), 4);
	});
}
