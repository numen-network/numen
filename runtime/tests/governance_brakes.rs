//! Governance brakes wiring. Referenda cancel and kill answer to the prime key
//! and to a track apiece, while the treasury reject origin stays prime alone.
//! Root is locked out of all of them.

mod common;

use common::new_test_ext;
use frame_support::{
	assert_noop, assert_ok,
	traits::{schedule::DispatchTime, tokens::fungible::Mutate, OriginTrait, StorePreimage},
};
use numen_runtime::{
	configs::{self, governance::pallet_custom_origins},
	AccountId, Balance, Balances, Bounties, Preimage, Referenda, Runtime, RuntimeCall,
	RuntimeOrigin, System, UNIT,
};
use pallet_referenda::{ReferendumInfo, ReferendumInfoFor};
use sp_keyring::Sr25519Keyring;
use sp_runtime::DispatchError;

const FUNDS: Balance = 10_000 * UNIT;

fn prime() -> AccountId {
	Sr25519Keyring::Ferdie.to_account_id()
}

fn install_prime() -> AccountId {
	let key = prime();
	pallet_prime::Key::<Runtime>::put(&key);
	key
}

fn assert_ongoing(index: u32) {
	assert!(
		matches!(ReferendumInfoFor::<Runtime>::get(index), Some(ReferendumInfo::Ongoing(_))),
		"referendum {index} must stay ongoing",
	);
}

/// Submit a small track referendum with its decision deposit placed, returning
/// its index. Both deposits stay reserved on the submitter. A qualified
/// identity goes straight into storage since submission is gated on one and
/// the gate has its own coverage.
fn ongoing_referendum(submitter: &AccountId) -> u32 {
	Balances::set_balance(submitter, FUNDS);
	pallet_identity::IdentityOf::<Runtime>::insert(
		submitter,
		pallet_identity::Registration {
			judgements: vec![(0, pallet_identity::Judgement::Reasonable)]
				.try_into()
				.expect("one judgement fits the bound"),
			deposit: 0,
			info: numen_runtime::identity_info::IdentityInfo {
				x: b"submitter".to_vec().try_into().expect("the handle fits the field bound"),
				..Default::default()
			},
		},
	);
	let index = pallet_referenda::ReferendumCount::<Runtime>::get();
	let proposal = <Preimage as StorePreimage>::bound(RuntimeCall::System(
		frame_system::Call::remark { remark: b"spend".to_vec() },
	))
	.expect("a remark call bounds inline");
	let track = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);
	assert_ok!(Referenda::submit(
		RuntimeOrigin::signed(submitter.clone()),
		Box::new(track.caller().clone()),
		proposal,
		DispatchTime::After(1),
	));
	assert_ok!(Referenda::place_decision_deposit(
		RuntimeOrigin::signed(submitter.clone()),
		index,
	));
	index
}

/// Place a bounty proposal, returning its id and the bond reserved for it.
fn proposed_bounty(proposer: &AccountId) -> (u32, Balance) {
	Balances::set_balance(proposer, FUNDS);
	let id = pallet_bounties::BountyCount::<Runtime>::get();
	assert_ok!(Bounties::propose_bounty(
		RuntimeOrigin::signed(proposer.clone()),
		1_000 * UNIT,
		b"work".to_vec(),
	));
	(id, Balances::reserved_balance(proposer))
}

#[test]
fn prime_cancels_referendum_and_deposits_stay_refundable() {
	new_test_ext().execute_with(|| {
		let key = install_prime();
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let index = ongoing_referendum(&submitter);

		assert_ok!(Referenda::cancel(RuntimeOrigin::signed(key), index));

		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Cancelled(..)),
		));
		assert_ok!(Referenda::refund_decision_deposit(
			RuntimeOrigin::signed(submitter.clone()),
			index,
		));
		assert_ok!(Referenda::refund_submission_deposit(
			RuntimeOrigin::signed(submitter.clone()),
			index,
		));
		assert_eq!(Balances::reserved_balance(&submitter), 0);
		assert_eq!(
			Balances::free_balance(&submitter),
			FUNDS,
			"a cancelled referendum returns both deposits in full",
		);
	});
}

#[test]
fn prime_kills_referendum_and_slashes_both_deposits() {
	new_test_ext().execute_with(|| {
		let key = install_prime();
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let treasury = configs::TreasuryAccount::get();
		let index = ongoing_referendum(&submitter);
		let deposits = Balances::reserved_balance(&submitter);
		assert!(deposits > 0);

		assert_ok!(Referenda::kill(RuntimeOrigin::signed(key), index));

		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Killed(_)),
		));
		assert_eq!(Balances::reserved_balance(&submitter), 0);
		assert_eq!(
			Balances::free_balance(&submitter),
			FUNDS - deposits,
			"a killed referendum forfeits both deposits",
		);
		assert_eq!(
			Balances::free_balance(&treasury),
			deposits,
			"slashed deposits land in the treasury pot",
		);
	});
}

#[test]
fn cancel_referendum_rejects_outsiders_and_root() {
	new_test_ext().execute_with(|| {
		install_prime();
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let index = ongoing_referendum(&submitter);
		let outsider = Sr25519Keyring::Bob.to_account_id();

		assert_noop!(
			Referenda::cancel(RuntimeOrigin::signed(outsider), index),
			DispatchError::BadOrigin,
		);
		assert_noop!(Referenda::cancel(RuntimeOrigin::root(), index), DispatchError::BadOrigin);

		assert_ongoing(index);
	});
}

#[test]
fn kill_referendum_rejects_outsiders_and_root() {
	new_test_ext().execute_with(|| {
		install_prime();
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let index = ongoing_referendum(&submitter);
		let outsider = Sr25519Keyring::Bob.to_account_id();

		assert_noop!(
			Referenda::kill(RuntimeOrigin::signed(outsider), index),
			DispatchError::BadOrigin,
		);
		assert_noop!(Referenda::kill(RuntimeOrigin::root(), index), DispatchError::BadOrigin);

		assert_ongoing(index);
	});
}

#[test]
fn prime_closes_proposed_bounty_and_slashes_bond() {
	new_test_ext().execute_with(|| {
		let key = install_prime();
		let proposer = Sr25519Keyring::Alice.to_account_id();
		let treasury = configs::TreasuryAccount::get();
		let (id, bond) = proposed_bounty(&proposer);
		assert!(bond > 0);

		assert_ok!(Bounties::close_bounty(RuntimeOrigin::signed(key), id));

		assert!(
			pallet_bounties::Bounties::<Runtime>::get(id).is_none(),
			"a rejected proposal is removed",
		);
		assert_eq!(Balances::reserved_balance(&proposer), 0);
		assert_eq!(
			Balances::free_balance(&proposer),
			FUNDS - bond,
			"the slashed bond is not returned",
		);
		assert_eq!(
			Balances::free_balance(&treasury),
			bond,
			"the slashed bond lands in the treasury pot",
		);
		System::assert_has_event(pallet_bounties::Event::BountyRejected { index: id, bond }.into());
	});
}

#[test]
fn close_bounty_rejects_non_prime_origins() {
	new_test_ext().execute_with(|| {
		install_prime();
		let proposer = Sr25519Keyring::Alice.to_account_id();
		let (id, _) = proposed_bounty(&proposer);
		let outsider = Sr25519Keyring::Bob.to_account_id();

		assert_noop!(
			Bounties::close_bounty(RuntimeOrigin::signed(outsider), id),
			DispatchError::BadOrigin,
		);
		assert_noop!(Bounties::close_bounty(RuntimeOrigin::root(), id), DispatchError::BadOrigin);

		assert!(
			pallet_bounties::Bounties::<Runtime>::get(id).is_some(),
			"the proposal survives rejected close attempts",
		);
	});
}

/// prime is not the only hand on the brake. A referendum won on the canceller
/// track stops another one without a prime signature anywhere.
#[test]
fn canceller_track_cancels_referendum() {
	new_test_ext().execute_with(|| {
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let index = ongoing_referendum(&submitter);

		assert_ok!(Referenda::cancel(
			RuntimeOrigin::from(pallet_custom_origins::Origin::ReferendumCanceller),
			index,
		));

		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Cancelled(..)),
		));
	});
}

#[test]
fn killer_track_kills_referendum_and_slashes_both_deposits() {
	new_test_ext().execute_with(|| {
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let index = ongoing_referendum(&submitter);
		let deposits = Balances::reserved_balance(&submitter);
		assert!(deposits > 0);

		assert_ok!(Referenda::kill(
			RuntimeOrigin::from(pallet_custom_origins::Origin::ReferendumKiller),
			index,
		));

		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Killed(_)),
		));
		assert_eq!(Balances::free_balance(&submitter), FUNDS - deposits);
	});
}

/// Cancel returns the deposits and kill takes them, so neither origin stands
/// in for the other.
#[test]
fn neither_brake_track_stands_in_for_the_other() {
	new_test_ext().execute_with(|| {
		let submitter = Sr25519Keyring::Alice.to_account_id();
		let index = ongoing_referendum(&submitter);

		assert_noop!(
			Referenda::cancel(
				RuntimeOrigin::from(pallet_custom_origins::Origin::ReferendumKiller),
				index,
			),
			DispatchError::BadOrigin,
		);
		assert_noop!(
			Referenda::kill(
				RuntimeOrigin::from(pallet_custom_origins::Origin::ReferendumCanceller),
				index,
			),
			DispatchError::BadOrigin,
		);

		assert_ongoing(index);
	});
}
