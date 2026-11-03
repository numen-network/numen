//! pallet-im-online heartbeats and offline reports. A heartbeat travels as an
//! authorized transaction, and a validator that cannot land one is reported
//! offline. A kicked or exiting validator stays in the set until the next
//! rotation, and a heartbeat it misses there must not be reported.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{assert_ok, traits::OneSessionHandler, WeakBoundedVec};
use frame_system::offchain::CreateAuthorizedTransaction;
use numen_runtime::{
	AccountId, Executive, ImOnline, Runtime, RuntimeCall, RuntimeEvent, System, UncheckedExtrinsic,
};
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use pallet_validator::{LockInfo, ValidatorLocks, ValidatorStatus};
use sp_core::Pair;
use sp_keyring::Sr25519Keyring;
use sp_runtime::transaction_validity::{
	InvalidTransaction, TransactionSource, TransactionValidityError,
};

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

fn register_im_online_keys(validators: &[Sr25519Keyring]) {
	let keys = validators.iter().map(|v| ImOnlineId::from(v.public())).collect();
	pallet_im_online::Keys::<Runtime>::put(WeakBoundedVec::force_from(keys, None));
}

/// The heartbeat an offchain worker submits for `authority_index`, signed by
/// `signer`.
fn heartbeat(signer: Sr25519Keyring, authority_index: u32) -> UncheckedExtrinsic {
	let heartbeat = pallet_im_online::Heartbeat {
		block_number: System::block_number(),
		session_index: pallet_session::CurrentIndex::<Runtime>::get(),
		authority_index,
		validators_len: pallet_im_online::Keys::<Runtime>::get().len() as u32,
	};
	let signature = signer.pair().sign(&heartbeat.encode()).into();
	let call = RuntimeCall::ImOnline(pallet_im_online::Call::heartbeat { heartbeat, signature });
	<Runtime as CreateAuthorizedTransaction<RuntimeCall>>::create_authorized_transaction(call)
}

#[test]
fn a_validator_heartbeat_passes_the_pool_and_lands_in_a_block() {
	new_test_ext().execute_with(|| {
		register_im_online_keys(&[Sr25519Keyring::Alice]);
		let session = pallet_session::CurrentIndex::<Runtime>::get();
		let xt = heartbeat(Sr25519Keyring::Alice, 0);

		assert_ok!(Executive::validate_transaction(
			TransactionSource::Local,
			xt.clone(),
			System::parent_hash(),
		));
		assert_ok!(Executive::apply_extrinsic(xt).expect("the block accepts the heartbeat"));

		assert!(pallet_im_online::ReceivedHeartbeats::<Runtime>::contains_key(session, 0));
	});
}

#[test]
fn a_heartbeat_signed_with_another_key_is_rejected() {
	new_test_ext().execute_with(|| {
		register_im_online_keys(&[Sr25519Keyring::Alice]);
		let session = pallet_session::CurrentIndex::<Runtime>::get();
		let forged = heartbeat(Sr25519Keyring::Bob, 0);
		let bad_proof = Err(TransactionValidityError::Invalid(InvalidTransaction::BadProof));

		assert_eq!(
			Executive::validate_transaction(
				TransactionSource::Local,
				forged.clone(),
				System::parent_hash(),
			)
			.map(|_| ()),
			bad_proof,
		);
		assert_eq!(Executive::apply_extrinsic(forged).map(|_| ()), bad_proof);

		assert!(!pallet_im_online::ReceivedHeartbeats::<Runtime>::contains_key(session, 0));
	});
}
