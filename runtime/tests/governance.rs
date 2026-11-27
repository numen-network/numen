//! Governance wiring. Bounty and child-bounty lifecycles run end to end under a
//! spender approval origin.

mod common;

use common::new_test_ext;
use frame_support::{
	assert_noop, assert_ok,
	traits::{tokens::fungible::Mutate, OnInitialize},
};
use pallet_bounties::BountyStatus;
use numen_runtime::{
	configs::{self, governance::pallet_custom_origins},
	AccountId, Balance, Balances, BlockNumber, Bounties, ChildBounties, Runtime, RuntimeOrigin,
	System, Treasury, UNIT,
};
use sp_keyring::Sr25519Keyring;
use sp_runtime::traits::StaticLookup;

type LookupSource = <<Runtime as frame_system::Config>::Lookup as StaticLookup>::Source;

const ACCOUNT_FUNDS: Balance = 10_000 * UNIT;
const TREASURY_FUNDS: Balance = 1_000_000 * UNIT;
const BOUNTY_VALUE: Balance = 1_000 * UNIT;
const BOUNTY_FEE: Balance = 100 * UNIT;
const CHILD_VALUE: Balance = 100 * UNIT;
const CHILD_FEE: Balance = 10 * UNIT;

fn acc(who: Sr25519Keyring) -> AccountId {
	who.to_account_id()
}

fn src(who: &AccountId) -> LookupSource {
	<Runtime as frame_system::Config>::Lookup::unlookup(who.clone())
}

fn payout_delay() -> BlockNumber {
	<Runtime as pallet_bounties::Config>::BountyDepositPayoutDelay::get()
}

/// Endow the given accounts and the treasury pot.
fn endow(accounts: &[&AccountId]) {
	for who in accounts {
		Balances::set_balance(who, ACCOUNT_FUNDS);
	}
	Balances::set_balance(&configs::TreasuryAccount::get(), TREASURY_FUNDS);
}

/// Drive the treasury to its next spend boundary so an approved bounty gets
/// funded from the pot.
fn fund_approved_bounties() {
	let spend_period = <Runtime as pallet_treasury::Config>::SpendPeriod::get();
	System::set_block_number(spend_period);
	Treasury::on_initialize(spend_period);
}

/// Take a parent bounty from proposal to an accepted curator, returning its id.
fn active_parent_bounty(proposer: &AccountId, curator: &AccountId, value: Balance, fee: Balance) -> u32 {
	let id = pallet_bounties::BountyCount::<Runtime>::get();
	let approve = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);
	assert_ok!(Bounties::propose_bounty(
		RuntimeOrigin::signed(proposer.clone()),
		value,
		b"parent bounty".to_vec(),
	));
	assert_ok!(Bounties::approve_bounty(approve.clone(), id));
	fund_approved_bounties();
	assert_ok!(Bounties::propose_curator(approve, id, src(curator), fee));
	assert_ok!(Bounties::accept_curator(RuntimeOrigin::signed(curator.clone()), id));
	id
}

#[test]
fn bounty_pays_beneficiary_and_curator_after_spender_approval() {
	let proposer = acc(Sr25519Keyring::Alice);
	let curator = acc(Sr25519Keyring::Bob);
	let beneficiary = acc(Sr25519Keyring::Charlie);
	let treasury = configs::TreasuryAccount::get();

	new_test_ext().execute_with(|| {
		endow(&[&proposer, &curator]);
		let treasury_before = Balances::free_balance(&treasury);

		let id = active_parent_bounty(&proposer, &curator, BOUNTY_VALUE, BOUNTY_FEE);
		assert_ok!(Bounties::award_bounty(
			RuntimeOrigin::signed(curator.clone()),
			id,
			src(&beneficiary),
		));

		// The payout unlocks on exactly `award block + BountyDepositPayoutDelay`.
		System::set_block_number(System::block_number() + payout_delay());
		assert_ok!(Bounties::claim_bounty(RuntimeOrigin::signed(proposer.clone()), id));

		assert_eq!(Balances::free_balance(&beneficiary), BOUNTY_VALUE - BOUNTY_FEE);
		assert_eq!(
			Balances::free_balance(&curator),
			ACCOUNT_FUNDS + BOUNTY_FEE,
			"curator keeps the fee and gets its deposit back",
		);
		assert_eq!(Balances::reserved_balance(&curator), 0);
		assert_eq!(
			Balances::free_balance(&proposer),
			ACCOUNT_FUNDS,
			"proposer bond is returned once the bounty is funded",
		);
		assert_eq!(Balances::reserved_balance(&proposer), 0);
		assert_eq!(
			treasury_before - Balances::free_balance(&treasury),
			BOUNTY_VALUE,
			"treasury pot funds the whole bounty value",
		);
		assert!(
			pallet_bounties::Bounties::<Runtime>::get(id).is_none(),
			"claimed bounty is removed from storage",
		);
	});
}

#[test]
fn claim_bounty_before_unlock_block_is_premature() {
	let proposer = acc(Sr25519Keyring::Alice);
	let curator = acc(Sr25519Keyring::Bob);
	let beneficiary = acc(Sr25519Keyring::Charlie);

	new_test_ext().execute_with(|| {
		endow(&[&proposer, &curator]);
		let id = active_parent_bounty(&proposer, &curator, BOUNTY_VALUE, BOUNTY_FEE);
		assert_ok!(Bounties::award_bounty(
			RuntimeOrigin::signed(curator.clone()),
			id,
			src(&beneficiary),
		));

		// One block short of the unlock block.
		System::set_block_number(System::block_number() + payout_delay() - 1);
		assert_noop!(
			Bounties::claim_bounty(RuntimeOrigin::signed(proposer), id),
			pallet_bounties::Error::<Runtime>::Premature,
		);

		assert_eq!(Balances::free_balance(&beneficiary), 0, "no payout before the unlock block");
	});
}

#[test]
fn award_bounty_by_non_curator_is_rejected() {
	let proposer = acc(Sr25519Keyring::Alice);
	let curator = acc(Sr25519Keyring::Bob);
	let intruder = acc(Sr25519Keyring::Dave);
	let beneficiary = acc(Sr25519Keyring::Charlie);

	new_test_ext().execute_with(|| {
		endow(&[&proposer, &curator, &intruder]);
		let id = active_parent_bounty(&proposer, &curator, BOUNTY_VALUE, BOUNTY_FEE);

		assert_noop!(
			Bounties::award_bounty(RuntimeOrigin::signed(intruder), id, src(&beneficiary)),
			pallet_bounties::Error::<Runtime>::RequireCurator,
		);

		let bounty = pallet_bounties::Bounties::<Runtime>::get(id).expect("bounty still exists");
		assert!(
			matches!(bounty.get_status(), BountyStatus::Active { .. }),
			"a rejected award must leave the bounty active",
		);
	});
}

#[test]
fn child_bounty_pays_beneficiary_without_new_vote() {
	let proposer = acc(Sr25519Keyring::Alice);
	let curator = acc(Sr25519Keyring::Bob);
	let child_curator = acc(Sr25519Keyring::Charlie);
	let child_beneficiary = acc(Sr25519Keyring::Dave);

	new_test_ext().execute_with(|| {
		endow(&[&proposer, &curator, &child_curator]);
		let parent = active_parent_bounty(&proposer, &curator, BOUNTY_VALUE, BOUNTY_FEE);
		assert_eq!(pallet_child_bounties::ParentChildBounties::<Runtime>::get(parent), 0);

		let child = pallet_child_bounties::ParentTotalChildBounties::<Runtime>::get(parent);
		assert_ok!(ChildBounties::add_child_bounty(
			RuntimeOrigin::signed(curator.clone()),
			parent,
			CHILD_VALUE,
			b"sub task".to_vec(),
		));
		assert_eq!(pallet_child_bounties::ParentChildBounties::<Runtime>::get(parent), 1);

		assert_ok!(ChildBounties::propose_curator(
			RuntimeOrigin::signed(curator),
			parent,
			child,
			src(&child_curator),
			CHILD_FEE,
		));
		assert_ok!(ChildBounties::accept_curator(
			RuntimeOrigin::signed(child_curator.clone()),
			parent,
			child,
		));
		assert_ok!(ChildBounties::award_child_bounty(
			RuntimeOrigin::signed(child_curator.clone()),
			parent,
			child,
			src(&child_beneficiary),
		));

		System::set_block_number(System::block_number() + payout_delay());
		assert_ok!(ChildBounties::claim_child_bounty(
			RuntimeOrigin::signed(proposer),
			parent,
			child,
		));

		assert_eq!(Balances::free_balance(&child_beneficiary), CHILD_VALUE - CHILD_FEE);
		assert_eq!(
			Balances::free_balance(&child_curator),
			ACCOUNT_FUNDS + CHILD_FEE,
			"child curator keeps the fee and gets its deposit back",
		);
		assert_eq!(Balances::reserved_balance(&child_curator), 0);
		assert_eq!(
			pallet_child_bounties::ParentChildBounties::<Runtime>::get(parent),
			0,
			"claiming releases the child bounty slot",
		);
	});
}

#[test]
fn add_child_bounty_enforces_value_minimum() {
	let proposer = acc(Sr25519Keyring::Alice);
	let curator = acc(Sr25519Keyring::Bob);

	new_test_ext().execute_with(|| {
		endow(&[&proposer, &curator]);
		let parent = active_parent_bounty(&proposer, &curator, BOUNTY_VALUE, BOUNTY_FEE);
		let minimum = <Runtime as pallet_child_bounties::Config>::ChildBountyValueMinimum::get();

		assert_noop!(
			ChildBounties::add_child_bounty(
				RuntimeOrigin::signed(curator.clone()),
				parent,
				minimum - 1,
				b"sub task".to_vec(),
			),
			pallet_bounties::Error::<Runtime>::InvalidValue,
		);

		assert_ok!(ChildBounties::add_child_bounty(
			RuntimeOrigin::signed(curator),
			parent,
			minimum,
			b"sub task".to_vec(),
		));
		assert_eq!(pallet_child_bounties::ParentChildBounties::<Runtime>::get(parent), 1);
	});
}
