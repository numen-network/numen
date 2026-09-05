//! Preimage terms. Noting bytes holds the flat base and the byte price, both
//! returned when the bytes are cleared, and a blob over the size ceiling is
//! refused whole. The stored ticket stays encoding compatible with the
//! HoldConsideration scheme it replaced.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{
	assert_noop, assert_ok,
	traits::tokens::fungible::{InspectHold, Mutate},
};
use numen_runtime::{
	configs::{
		PreimageTicket, TreasuryAccount, PREIMAGE_BASE_DEPOSIT, PREIMAGE_BYTE_DEPOSIT,
		PREIMAGE_MAX_SIZE,
	},
	AccountId, Balance, Balances, Preimage, Runtime, RuntimeHoldReason, RuntimeOrigin,
	EXISTENTIAL_DEPOSIT, UNIT,
};
use sp_keyring::Sr25519Keyring;
use sp_runtime::{traits::{BlakeTwo256, Hash}, DispatchError, TokenError};

const NOTER_FUNDS: Balance = 100_000 * UNIT;
const LEN: usize = 1_000;

fn noter() -> AccountId {
	Sr25519Keyring::Alice.to_account_id()
}

fn hold_reason() -> RuntimeHoldReason {
	RuntimeHoldReason::Preimage(pallet_preimage::HoldReason::Preimage)
}

fn held_for(len: usize) -> Balance {
	PREIMAGE_BASE_DEPOSIT + PREIMAGE_BYTE_DEPOSIT * len as Balance
}

fn note(who: &AccountId, bytes: Vec<u8>) -> Result<(), DispatchError> {
	Preimage::note_preimage(RuntimeOrigin::signed(who.clone()), bytes)
		.map(|_| ())
		.map_err(|e| e.error)
}

#[test]
fn noting_holds_the_base_and_the_byte_price_and_spends_nothing() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, NOTER_FUNDS);

		assert_ok!(note(&who, vec![7u8; LEN]));

		assert_eq!(Balances::balance_on_hold(&hold_reason(), &who), held_for(LEN));
		assert_eq!(Balances::free_balance(&who), NOTER_FUNDS - held_for(LEN));
		assert_eq!(Balances::free_balance(TreasuryAccount::get()), 0);
	});
}

#[test]
fn clearing_returns_the_whole_hold() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, NOTER_FUNDS);
		let bytes = vec![7u8; LEN];
		let hash = BlakeTwo256::hash(&bytes);
		assert_ok!(note(&who, bytes));

		assert_ok!(Preimage::unnote_preimage(RuntimeOrigin::signed(who.clone()), hash));

		assert_eq!(Balances::balance_on_hold(&hold_reason(), &who), 0);
		assert_eq!(Balances::free_balance(&who), NOTER_FUNDS);
	});
}

#[test]
fn an_exactly_funded_noter_covers_the_whole_hold() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, held_for(LEN) + EXISTENTIAL_DEPOSIT);

		assert_ok!(note(&who, vec![7u8; LEN]));

		assert_eq!(Balances::free_balance(&who), EXISTENTIAL_DEPOSIT);
		assert_eq!(Balances::balance_on_hold(&hold_reason(), &who), held_for(LEN));
	});
}

#[test]
fn a_noter_one_planck_short_of_the_hold_is_refused() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, held_for(LEN) + EXISTENTIAL_DEPOSIT - 1);

		assert_noop!(note(&who, vec![7u8; LEN]), TokenError::FundsUnavailable);

		assert_eq!(Balances::balance_on_hold(&hold_reason(), &who), 0);
	});
}

#[test]
fn a_preimage_at_the_size_ceiling_is_accepted() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, NOTER_FUNDS);

		assert_ok!(note(&who, vec![7u8; PREIMAGE_MAX_SIZE as usize]));

		assert_eq!(
			Balances::balance_on_hold(&hold_reason(), &who),
			held_for(PREIMAGE_MAX_SIZE as usize),
		);
	});
}

#[test]
fn a_preimage_over_the_size_ceiling_is_refused() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, NOTER_FUNDS);

		assert_noop!(
			note(&who, vec![7u8; PREIMAGE_MAX_SIZE as usize + 1]),
			DispatchError::Exhausted
		);

		assert_eq!(Balances::balance_on_hold(&hold_reason(), &who), 0);
	});
}

/// Tickets written by HoldConsideration held the same linear price as one
/// bare balance. The replacement must read those and write nothing wider, or
/// every live ticket would need a migration.
#[test]
fn a_ticket_encodes_as_the_bare_balance_the_old_scheme_wrote() {
	new_test_ext().execute_with(|| {
		let who = noter();
		Balances::set_balance(&who, NOTER_FUNDS);
		let bytes = vec![7u8; LEN];
		let hash = BlakeTwo256::hash(&bytes);
		assert_ok!(note(&who, bytes));

		let status = pallet_preimage::RequestStatusFor::<Runtime>::get(hash)
			.expect("the preimage was noted");
		let pallet_preimage::RequestStatus::Unrequested { ticket: (owner, ticket), .. } = status
		else {
			panic!("nothing requested it, so it sits unrequested");
		};

		assert_eq!(owner, who);
		let ticket: PreimageTicket = ticket;
		assert_eq!(ticket.encode(), held_for(LEN).encode());
	});
}
