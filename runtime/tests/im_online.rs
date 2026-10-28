//! pallet-im-online offline reports. A kicked or exiting validator stays in
//! the set until the next rotation, and a heartbeat it misses there must not
//! be reported.

mod common;

use common::new_test_ext;
use frame_support::traits::OneSessionHandler;
use numen_runtime::{AccountId, ImOnline, Runtime, RuntimeEvent, System};
use pallet_validator::{LockInfo, ValidatorLocks, ValidatorStatus};
use sp_keyring::Sr25519Keyring;

fn seat(who: &AccountId, status: ValidatorStatus) {
	ValidatorLocks::<Runtime>::insert(
		who,
		LockInfo { amount: 0, lock_block: 0, expiry_block: 1_000, status },
	);
}

fn end_session() {
	<ImOnline as OneSessionHandler<AccountId>>::on_before_session_ending();
}

fn offline_report() -> Option<Vec<AccountId>> {
	System::events().into_iter().find_map(|r| match r.event {
		RuntimeEvent::ImOnline(pallet_im_online::Event::SomeOffline { offline }) => {
			Some(offline.into_iter().map(|(who, _)| who).collect())
		},
		_ => None,
	})
}

fn all_good() -> bool {
	System::events()
		.into_iter()
		.any(|r| matches!(r.event, RuntimeEvent::ImOnline(pallet_im_online::Event::AllGood)))
}

#[test]
fn offline_report_names_only_active_validators() {
	new_test_ext().execute_with(|| {
		let active = Sr25519Keyring::Alice.to_account_id();
		let exiting = Sr25519Keyring::Bob.to_account_id();
		let kicked = Sr25519Keyring::Charlie.to_account_id();
		seat(&active, ValidatorStatus::Active);
		seat(&exiting, ValidatorStatus::ExitRequested);
		seat(&kicked, ValidatorStatus::Kicked);
		pallet_session::Validators::<Runtime>::put(vec![
			active.clone(),
			exiting.clone(),
			kicked.clone(),
		]);

		end_session();

		assert_eq!(offline_report(), Some(vec![active.clone()]));
		assert!(pallet_validator::OfflineThisSession::<Runtime>::contains_key(&active));
		assert!(!pallet_validator::OfflineThisSession::<Runtime>::contains_key(&exiting));
		assert!(!pallet_validator::OfflineThisSession::<Runtime>::contains_key(&kicked));
	});
}

#[test]
fn session_is_all_good_when_only_departing_validators_miss() {
	new_test_ext().execute_with(|| {
		let active = Sr25519Keyring::Alice.to_account_id();
		let kicked = Sr25519Keyring::Charlie.to_account_id();
		seat(&active, ValidatorStatus::Active);
		seat(&kicked, ValidatorStatus::Kicked);
		pallet_session::Validators::<Runtime>::put(vec![active.clone(), kicked]);
		pallet_im_online::AuthoredBlocks::<Runtime>::insert(
			pallet_session::CurrentIndex::<Runtime>::get(),
			&active,
			1,
		);

		end_session();

		assert_eq!(offline_report(), None);
		assert!(all_good());
	});
}
