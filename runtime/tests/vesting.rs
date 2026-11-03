//! pallet-vesting wired for the airdrop. A vested transfer locks the grant and
//! releases it linearly per block. These pin the release curve, the completion
//! that clears the lock, and the lock holding firm against fees.

mod common;

use common::new_test_ext;
use frame_support::{
	assert_noop, assert_ok,
	traits::{tokens::fungible::Mutate, Currency, ExistenceRequirement, WithdrawReasons},
};
use numen_runtime::{AccountId, Balances, Runtime, RuntimeOrigin, System, Vesting, UNIT};
use pallet_vesting::VestingInfo;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{traits::StaticLookup, TokenError};

/// 100 NUMN released over 100 blocks, one NUMN per block, starting at genesis
/// of the test (block 1). Every boundary below reads off this line.
const GRANT: u128 = 100 * UNIT;
const PER_BLOCK: u128 = UNIT;
const START: u32 = 1;

fn src(who: &AccountId) -> <<Runtime as frame_system::Config>::Lookup as StaticLookup>::Source {
	<Runtime as frame_system::Config>::Lookup::unlookup(who.clone())
}

/// Fund `source` and vest `GRANT` onto `target` with the linear schedule.
fn airdrop_to(source: &AccountId, target: &AccountId) {
	Balances::set_balance(source, 2 * GRANT);
	assert_ok!(Vesting::vested_transfer(
		RuntimeOrigin::signed(source.clone()),
		src(target),
		VestingInfo::new(GRANT, PER_BLOCK, START),
	));
}

#[test]
fn vest_frees_exactly_the_elapsed_share() {
	new_test_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let bob = Sr25519Keyring::Bob.to_account_id();
		let sink = Sr25519Keyring::Charlie.to_account_id();
		airdrop_to(&alice, &bob);

		// At the starting block the whole grant is locked, not one planck moves.
		assert_noop!(
			Balances::transfer_keep_alive(RuntimeOrigin::signed(bob.clone()), src(&sink), UNIT),
			TokenError::Frozen,
		);

		// Halfway along the line exactly half has vested. `vest` rewrites the
		// lock to the elapsed share; a wei past it is still frozen.
		System::set_block_number(START + 50);
		assert_ok!(Vesting::vest(RuntimeOrigin::signed(bob.clone())));
		let released = 50 * PER_BLOCK;
		assert_noop!(
			Balances::transfer_keep_alive(
				RuntimeOrigin::signed(bob.clone()),
				src(&sink),
				released + 1,
			),
			TokenError::Frozen,
		);
		assert_ok!(Balances::transfer_keep_alive(
			RuntimeOrigin::signed(bob.clone()),
			src(&sink),
			released,
		));
		assert_eq!(Balances::free_balance(&sink), released);
	});
}

#[test]
fn vest_completes_and_clears_schedule_after_full_term() {
	new_test_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let bob = Sr25519Keyring::Bob.to_account_id();
		let sink = Sr25519Keyring::Charlie.to_account_id();
		airdrop_to(&alice, &bob);

		System::set_block_number(START + 100);
		assert_ok!(Vesting::vest(RuntimeOrigin::signed(bob.clone())));

		System::assert_last_event(
			pallet_vesting::Event::VestingCompleted { account: bob.clone() }.into(),
		);
		assert!(Vesting::vesting(bob.clone()).is_none(), "schedule cleared once fully vested");

		// The lock is gone, so the entire grant is transferable.
		assert_ok!(Balances::transfer_allow_death(
			RuntimeOrigin::signed(bob.clone()),
			src(&sink),
			GRANT,
		));
		assert_eq!(Balances::free_balance(&sink), GRANT);
	});
}

/// The reason set on the vesting lock reads as though fees might slip through.
/// They cannot. Pinned so a future edit to that set does not quietly turn into
/// a real exemption.
#[test]
fn fees_cannot_draw_on_the_locked_grant() {
	new_test_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let bob = Sr25519Keyring::Bob.to_account_id();
		airdrop_to(&alice, &bob);

		let fee = |amount: u128| {
			<Balances as Currency<AccountId>>::withdraw(
				&bob,
				amount,
				WithdrawReasons::TRANSACTION_PAYMENT,
				ExistenceRequirement::KeepAlive,
			)
		};

		assert_noop!(fee(1), pallet_balances::Error::<Runtime>::LiquidityRestrictions);

		// The elapsed share is the whole fee budget, down to the last planck.
		System::set_block_number(START + 50);
		assert_ok!(Vesting::vest(RuntimeOrigin::signed(bob.clone())));
		let released = 50 * PER_BLOCK;
		assert_noop!(fee(released + 1), pallet_balances::Error::<Runtime>::LiquidityRestrictions);
		assert_ok!(fee(released));
	});
}
