// Replay protection for signed Substrate extrinsics.
//
// An account keeps a provider of its own from its first signed transaction, so
// a drain leaves it in state and its nonce never rewinds to zero. Here the
// sender empties itself with `transfer_all` yet keeps its nonce, so the nonce
// check rejects the same immortal extrinsic as stale once the address holds
// funds again.

mod common;

use common::{new_test_ext, signed_immortal};
use frame_support::traits::tokens::fungible::Mutate;
use numen_runtime::{AccountId, Balances, Executive, RuntimeCall, System, UncheckedExtrinsic, UNIT};
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	transaction_validity::{InvalidTransaction, TransactionValidityError},
	MultiAddress,
};

const FUNDING: u128 = 5_000 * UNIT;

fn transfer_all_to(dest: &AccountId) -> RuntimeCall {
	RuntimeCall::Balances(pallet_balances::Call::transfer_all {
		dest: MultiAddress::Id(dest.clone()),
		keep_alive: false,
	})
}

/// Fund the signer, apply its nonce-0 drain into `recipient` and return the
/// signed bytes an attacker would replay.
fn drain(signer: Sr25519Keyring, recipient: &AccountId) -> UncheckedExtrinsic {
	Balances::set_balance(&signer.to_account_id(), FUNDING);
	let drain = signed_immortal(signer, 0, transfer_all_to(recipient));
	Executive::apply_extrinsic(drain.clone())
		.expect("the drain is valid")
		.expect("the drain dispatches");
	assert_eq!(Balances::free_balance(signer.to_account_id()), 0, "the drain emptied the sender");
	drain
}

#[test]
fn drained_sender_keeps_its_nonce_so_the_immortal_extrinsic_cannot_replay() {
	new_test_ext().execute_with(|| {
		let sender = Sr25519Keyring::Ferdie;
		let sender_acc = sender.to_account_id();
		let recipient = Sr25519Keyring::Dave.to_account_id();
		let drain = drain(sender, &recipient);
		let paid = Balances::free_balance(&recipient);

		assert!(System::account_exists(&sender_acc), "the drained account stays in state");
		assert_eq!(
			System::account_nonce(&sender_acc),
			1,
			"the drained sender keeps its advanced nonce"
		);

		Balances::set_balance(&sender_acc, FUNDING);
		assert_eq!(
			Executive::apply_extrinsic(drain),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
			"the block rejects the replayed extrinsic as stale"
		);
		assert_eq!(Balances::free_balance(&sender_acc), FUNDING, "the rejected replay costs nothing");
		assert_eq!(Balances::free_balance(&recipient), paid, "the recipient is paid once");
	});
}

#[test]
fn drained_sender_spends_again_with_its_next_nonce() {
	new_test_ext().execute_with(|| {
		let sender = Sr25519Keyring::Ferdie;
		let sender_acc = sender.to_account_id();
		let recipient = Sr25519Keyring::Dave.to_account_id();
		drain(sender, &recipient);
		let paid = Balances::free_balance(&recipient);

		Balances::set_balance(&sender_acc, FUNDING);
		Executive::apply_extrinsic(signed_immortal(sender, 1, transfer_all_to(&recipient)))
			.expect("the next nonce is valid")
			.expect("the next nonce dispatches");
		assert!(Balances::free_balance(&recipient) > paid, "the refilled account spends again");
		assert_eq!(System::account_nonce(&sender_acc), 2);
	});
}
