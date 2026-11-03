use crate::{mock::*, Event, Pallet, WeightInfo, NOTHING_TO_CLAIM, NO_ROOM_FOR_VESTING};
use frame_support::{
	assert_ok,
	traits::{
		fungible::{Inspect, Mutate},
		tokens::{Fortitude, Precision, Preservation},
		Authorize, LockableCurrency, ReservableCurrency, VestingSchedule, WithdrawReasons,
	},
};
use frame_system::RawOrigin;
use sp_core::H160;
use sp_runtime::{
	transaction_validity::{InvalidTransaction, TransactionSource, TransactionValidityError},
	AccountId32,
};

fn address() -> H160 {
	H160::repeat_byte(0x11)
}

fn legacy() -> AccountId32 {
	Pallet::<Test>::legacy_account(&address())
}

fn account() -> AccountId32 {
	Pallet::<Test>::revive_account(&address())
}

fn claim() {
	assert_ok!(EvmClaims::claim(RawOrigin::Authorized.into(), address()));
}

fn authorize() -> Result<(), TransactionValidityError> {
	RuntimeCall::EvmClaims(crate::Call::claim { address: address() })
		.authorize(TransactionSource::External)
		.expect("claim carries authorize logic")
		.map(|_| ())
}

fn set_nonce(who: &AccountId32, nonce: u32) {
	frame_system::Account::<Test>::mutate(who, |info| info.nonce = nonce);
}

fn spendable(who: &AccountId32) -> Balance {
	Balances::reducible_balance(who, Preservation::Preserve, Fortitude::Polite)
}

fn fee() -> Balance {
	<Test as crate::Config>::ClaimFee::get()
}

fn refused(code: u8) -> Result<(), TransactionValidityError> {
	Err(InvalidTransaction::Custom(code).into())
}

#[test]
fn the_legacy_account_is_the_frontier_derivation() {
	let alith = H160(hex_literal::hex!("f24ff3a9cf04c71dbc94d0b566f7a27b94566cac"));

	assert_eq!(
		Pallet::<Test>::legacy_account(&alith),
		AccountId32::new(hex_literal::hex!(
			"a02a00e549cb104f710d3fe6f2f83e91524d2a40c4ed831658a120883077f9a9"
		)),
	);
	assert_eq!(
		Pallet::<Test>::revive_account(&alith),
		AccountId32::new(hex_literal::hex!(
			"f24ff3a9cf04c71dbc94d0b566f7a27b94566caceeeeeeeeeeeeeeeeeeeeeeee"
		)),
	);
}

/// What the new account can spend afterwards is what the old one held less the
/// fee, which goes where the runtime sends fees. The existential deposit the
/// new account needs stays out of the active issuance.
#[test]
fn a_claim_hands_over_the_old_balance_less_its_fee() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 500);
		let issuance = Balances::total_issuance();
		let active = Balances::active_issuance();

		claim();

		assert_eq!(spendable(&account()), 500 - fee());
		assert_eq!(FeesTaken::get(), fee());
		assert_eq!(Balances::total_issuance(), issuance + 1 - fee());
		assert_eq!(Balances::active_issuance(), active - fee());
		assert!(!frame_system::Account::<Test>::contains_key(legacy()));
		System::assert_has_event(
			Event::Claimed { address: address(), account: account(), amount: 500, nonce: 0 }.into(),
		);
	});
}

#[test]
fn a_claim_adds_to_funds_already_at_the_new_account() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&account(), 100);
		Balances::set_balance(&legacy(), 500);
		let issuance = Balances::total_issuance();

		claim();

		assert_eq!(Balances::balance(&account()), 600 - fee());
		assert_eq!(Balances::total_issuance(), issuance - fee());
	});
}

/// A balance has to be worth more than the fee to be claimed, so nobody can
/// buy a claim's block space for a plank.
#[test]
fn a_balance_worth_no_more_than_the_fee_is_not_claimable() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), fee());
		assert_eq!(authorize(), refused(NOTHING_TO_CLAIM));

		Balances::set_balance(&legacy(), fee() + 1);
		assert_ok!(authorize());
		claim();

		assert_eq!(spendable(&account()), 1);
		assert_eq!(FeesTaken::get(), fee());
	});
}

/// Funds a vesting schedule still locks pay no fee and do not make a balance
/// claimable.
#[test]
fn locked_funds_count_for_neither_the_fee_nor_the_claim() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 1_000);
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 1_000 - fee(), 1, 100));
		assert_eq!(authorize(), refused(NOTHING_TO_CLAIM));

		assert_ok!(Vesting::remove_vesting_schedule(&legacy(), 0));
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 1_000 - fee() - 1, 1, 100));
		assert_ok!(authorize());
		claim();

		assert_eq!(Vesting::vesting_balance(&account()), Some(1_000 - fee() - 1));
		assert_eq!(Balances::balance(&account()), 1 + 1_000 - fee());
		assert_eq!(FeesTaken::get(), fee());
	});
}

/// A claim the nonce alone admits still pays, but never more than the funds it
/// brings along. What the new account already holds stays untouched.
#[test]
fn the_fee_comes_only_out_of_what_the_claim_moves() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&account(), 100);
		Balances::set_balance(&legacy(), fee() - 1);
		set_nonce(&legacy(), 5);

		assert_ok!(authorize());
		claim();

		assert_eq!(FeesTaken::get(), fee() - 1);
		assert_eq!(Balances::balance(&account()), 100);
		assert_eq!(System::account_nonce(account()), 5);
	});
}

/// The block keeps only the weight of the schedules a claim moved, so a claim
/// without vesting does not fill it as if it had moved the maximum.
#[test]
fn a_claim_reports_the_weight_of_the_schedules_it_moved() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 1_000);
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 100, 1, 1));
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 100, 1, 1));

		let post = EvmClaims::claim(RawOrigin::Authorized.into(), address()).unwrap();

		assert_eq!(post.actual_weight, Some(<() as WeightInfo>::claim(2)));
		assert!(<() as WeightInfo>::claim(2).all_lt(<() as WeightInfo>::claim(
			<Test as pallet_vesting::Config>::MAX_VESTING_SCHEDULES,
		)));
	});
}

/// The schedules a claim moves have to fit next to the ones the new account
/// already has, or the claim would fail after the pool let it through.
#[test]
fn a_claim_whose_schedules_do_not_fit_is_refused() {
	new_test_ext().execute_with(|| {
		let max = <Test as pallet_vesting::Config>::MAX_VESTING_SCHEDULES;
		Balances::set_balance(&account(), 10_000);
		for _ in 0..max {
			assert_ok!(Vesting::add_vesting_schedule(&account(), 10, 1, 1_000));
		}
		Balances::set_balance(&legacy(), 500);
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 100, 1, 1_000));
		assert_eq!(authorize(), refused(NO_ROOM_FOR_VESTING));

		assert_ok!(Vesting::remove_vesting_schedule(&account(), 0));
		assert_ok!(authorize());
		claim();

		assert_eq!(
			pallet_vesting::Vesting::<Test>::get(account()).unwrap().len() as u32,
			max,
		);
	});
}

#[test]
fn vesting_moves_along_and_unlocks_on_the_same_schedule() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 1_000);
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 600, 10, 1));
		System::set_block_number(11);
		let locked_then = Vesting::vesting_balance(&legacy());

		claim();

		assert_eq!(Vesting::vesting_balance(&account()), locked_then);
		assert_eq!(
			pallet_vesting::Vesting::<Test>::get(account()).unwrap().into_inner(),
			vec![pallet_vesting::VestingInfo::new(600, 10, 1)],
		);
		System::set_block_number(41);
		assert_eq!(Vesting::vesting_balance(&account()), Some(600 - 40 * 10));
		assert!(pallet_vesting::Vesting::<Test>::get(legacy()).is_none());
	});
}

#[test]
fn a_finished_schedule_is_not_carried() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 1_000);
		assert_ok!(Vesting::add_vesting_schedule(&legacy(), 100, 10, 1));
		System::set_block_number(20);

		claim();

		assert!(pallet_vesting::Vesting::<Test>::get(account()).is_none());
		assert_eq!(spendable(&account()), 1_000 - fee());
	});
}

/// The new account keeps the nonce through a drain, so the transactions
/// Frontier ran for the H160 stay spent.
#[test]
fn the_nonce_moves_and_survives_a_drain() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 500);
		set_nonce(&legacy(), 5);

		claim();

		assert_eq!(System::account_nonce(account()), 5);
		let all = Balances::balance(&account());
		assert_ok!(Balances::burn_from(
			&account(),
			all,
			Preservation::Expendable,
			Precision::Exact,
			Fortitude::Force,
		));
		assert!(System::account_exists(&account()));
		assert_eq!(System::account_nonce(account()), 5);
	});
}

#[test]
fn a_higher_nonce_at_the_new_account_stays() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&account(), 10);
		set_nonce(&account(), 9);
		Balances::set_balance(&legacy(), 500);
		set_nonce(&legacy(), 5);

		claim();

		assert_eq!(System::account_nonce(account()), 9);
	});
}

/// Nobody can manufacture a nonce, so a nonce moves for free.
#[test]
fn a_nonce_alone_moves_onto_an_empty_account_for_free() {
	new_test_ext().execute_with(|| {
		System::inc_providers(&legacy());
		set_nonce(&legacy(), 3);

		assert_ok!(authorize());
		claim();

		assert_eq!(System::account_nonce(account()), 3);
		assert!(System::account_exists(&account()));
		assert!(!frame_system::Account::<Test>::contains_key(legacy()));
		assert_eq!(FeesTaken::get(), 0);
	});
}

/// Funds under a reserve or a lock other than vesting are for governance to
/// sort out. Holds count as reserves. The nonce moves regardless.
#[test]
fn encumbered_funds_stay_and_only_the_nonce_moves() {
	let encumber: [fn(&AccountId32); 2] = [
		|who| Balances::set_lock(*b"staking ", who, 100, WithdrawReasons::all()),
		|who| {
			assert_ok!(Balances::reserve(who, 100));
		},
	];
	for encumber in encumber {
		new_test_ext().execute_with(|| {
			Balances::set_balance(&legacy(), 500);
			set_nonce(&legacy(), 4);
			encumber(&legacy());

			claim();

			assert_eq!(Balances::total_balance(&legacy()), 500);
			assert_eq!(Balances::total_balance(&account()), 0);
			assert_eq!(System::account_nonce(account()), 4);
			assert_eq!(FeesTaken::get(), 0);
		});
	}
}

#[test]
fn a_claim_with_nothing_to_move_is_refused() {
	new_test_ext().execute_with(|| {
		assert_eq!(authorize(), refused(NOTHING_TO_CLAIM));

		Balances::set_balance(&legacy(), 500);
		set_nonce(&legacy(), 2);
		assert_ok!(authorize());
		claim();

		assert_eq!(authorize(), refused(NOTHING_TO_CLAIM));
	});
}

/// Funds sent to the old account after a claim can be claimed again, each
/// claim paying its own fee.
#[test]
fn a_later_payment_to_the_old_account_is_claimed_again() {
	new_test_ext().execute_with(|| {
		Balances::set_balance(&legacy(), 500);
		claim();
		Balances::set_balance(&legacy(), 70);

		assert_ok!(authorize());
		claim();

		assert_eq!(spendable(&account()), 570 - 2 * fee());
		assert_eq!(FeesTaken::get(), 2 * fee());
	});
}

#[test]
fn lift_nonce_raises_an_ethereum_signer_that_never_signed() {
	new_test_ext().execute_with(|| {
		set_nonce(&legacy(), 6);

		Pallet::<Test>::lift_nonce(&account());

		assert_eq!(System::account_nonce(account()), 6);
		assert!(System::account_exists(&account()));
	});
}

#[test]
fn lift_nonce_leaves_signers_it_does_not_cover() {
	new_test_ext().execute_with(|| {
		set_nonce(&legacy(), 6);
		Balances::set_balance(&account(), 10);
		set_nonce(&account(), 1);
		let substrate = AccountId32::new([7; 32]);

		Pallet::<Test>::lift_nonce(&account());
		Pallet::<Test>::lift_nonce(&substrate);

		assert_eq!(System::account_nonce(account()), 1);
		assert_eq!(System::account_nonce(substrate), 0);
	});
}
