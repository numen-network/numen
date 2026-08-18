//! Referendum submission gate. Only accounts backed by a qualified identity
//! may open referenda. Qualified means judged Reasonable or KnownGood while
//! carrying at least one plaintext social channel among x, telegram and
//! discord, and a sub account qualifies through its parent.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{
	assert_noop, assert_ok,
	traits::{schedule::DispatchTime, tokens::fungible::Mutate, Bounded},
};
use numen_runtime::{
	configs::governance::pallet_custom_origins, identity_info::IdentityInfo, AccountId, Balance,
	Balances, Identity, Referenda, Runtime, RuntimeCall, RuntimeOrigin, UNIT,
};
use pallet_identity::{Data, Judgement};
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	traits::{Hash, StaticLookup},
	DispatchError, DispatchResult,
};

const FUNDS: Balance = 10_000 * UNIT;

type IdInfo = <Runtime as pallet_identity::Config>::IdentityInformation;

fn src(who: &AccountId) -> <<Runtime as frame_system::Config>::Lookup as StaticLookup>::Source {
	<Runtime as frame_system::Config>::Lookup::unlookup(who.clone())
}

fn funded(keyring: Sr25519Keyring) -> AccountId {
	let who = keyring.to_account_id();
	Balances::set_balance(&who, FUNDS);
	who
}

fn raw(bytes: &[u8]) -> Data {
	Data::Raw(bytes.to_vec().try_into().expect("raw data fits the field bound"))
}

fn identity_info(x: Data) -> IdInfo {
	IdentityInfo { display: raw(b"proposer"), x, ..Default::default() }
}

/// Installs Eve as registrar zero through the prime key.
fn install_registrar() -> AccountId {
	let prime = Sr25519Keyring::Ferdie.to_account_id();
	pallet_prime::Key::<Runtime>::put(&prime);
	let registrar = funded(Sr25519Keyring::Eve);
	assert_ok!(Identity::add_registrar(
		RuntimeOrigin::signed(prime),
		src(&registrar),
	));
	registrar
}

/// Registers `info` for `who` and judges it `judgement` through registrar zero.
fn judged_identity(who: &AccountId, judgement: Judgement<Balance>, info: IdInfo) {
	let registrar = install_registrar();
	assert_ok!(Identity::set_identity(
		RuntimeOrigin::signed(who.clone()),
		Box::new(info.clone()),
	));
	assert_ok!(Identity::provide_judgement(
		RuntimeOrigin::signed(registrar),
		0,
		src(who),
		judgement,
		<Runtime as frame_system::Config>::Hashing::hash_of(&info),
	));
}

fn submit(who: &AccountId) -> DispatchResult {
	let call = RuntimeCall::System(frame_system::Call::remark { remark: vec![] });
	Referenda::submit(
		RuntimeOrigin::signed(who.clone()),
		Box::new(pallet_custom_origins::Origin::SmallSpender.into()),
		Bounded::Inline(call.encode().try_into().expect("remark fits the inline bound")),
		DispatchTime::After(0),
	)
}

fn referendum_count() -> u32 {
	pallet_referenda::ReferendumCount::<Runtime>::get()
}

#[test]
fn plain_account_cannot_submit() {
	new_test_ext().execute_with(|| {
		let who = funded(Sr25519Keyring::Alice);

		assert_noop!(submit(&who), DispatchError::BadOrigin);
		assert_eq!(referendum_count(), 0);
	});
}

#[test]
fn unjudged_identity_cannot_submit() {
	new_test_ext().execute_with(|| {
		let who = funded(Sr25519Keyring::Alice);
		assert_ok!(Identity::set_identity(
			RuntimeOrigin::signed(who.clone()),
			Box::new(identity_info(raw(b"@proposer"))),
		));

		assert_noop!(submit(&who), DispatchError::BadOrigin);
		assert_eq!(referendum_count(), 0);
	});
}

#[test]
fn negative_judgement_cannot_submit() {
	new_test_ext().execute_with(|| {
		let who = funded(Sr25519Keyring::Alice);
		judged_identity(&who, Judgement::OutOfDate, identity_info(raw(b"@proposer")));

		assert_noop!(submit(&who), DispatchError::BadOrigin);
		assert_eq!(referendum_count(), 0);
	});
}

#[test]
fn judged_identity_without_qualifying_channel_cannot_submit() {
	new_test_ext().execute_with(|| {
		let who = funded(Sr25519Keyring::Alice);
		let info = IdentityInfo {
			display: raw(b"proposer"),
			web: raw(b"proposer.example"),
			email: raw(b"proposer@example.com"),
			matrix: raw(b"@proposer:example.com"),
			github: raw(b"proposer"),
			..Default::default()
		};
		judged_identity(&who, Judgement::Reasonable, info);

		assert_noop!(submit(&who), DispatchError::BadOrigin);
		assert_eq!(referendum_count(), 0);
	});
}

#[test]
fn non_plaintext_social_channel_cannot_submit() {
	let commitment = <Runtime as frame_system::Config>::Hashing::hash(b"@proposer").0;
	let hashed_x = identity_info(Data::BlakeTwo256(commitment));
	let empty_telegram =
		IdentityInfo { display: raw(b"proposer"), telegram: raw(b""), ..Default::default() };

	for info in [hashed_x, empty_telegram] {
		new_test_ext().execute_with(|| {
			let who = funded(Sr25519Keyring::Alice);
			judged_identity(&who, Judgement::Reasonable, info.clone());

			assert_noop!(submit(&who), DispatchError::BadOrigin);
			assert_eq!(referendum_count(), 0);
		});
	}
}

#[test]
fn reasonable_judgement_submits() {
	new_test_ext().execute_with(|| {
		let who = funded(Sr25519Keyring::Alice);
		judged_identity(&who, Judgement::Reasonable, identity_info(raw(b"@proposer")));

		assert_ok!(submit(&who));
		assert_eq!(referendum_count(), 1);
	});
}

#[test]
fn known_good_judgement_submits() {
	new_test_ext().execute_with(|| {
		let who = funded(Sr25519Keyring::Alice);
		judged_identity(&who, Judgement::KnownGood, identity_info(raw(b"@proposer")));

		assert_ok!(submit(&who));
		assert_eq!(referendum_count(), 1);
	});
}

#[test]
fn telegram_and_discord_channels_qualify() {
	let handle = raw(b"@proposer");
	let telegram =
		IdentityInfo { display: raw(b"proposer"), telegram: handle.clone(), ..Default::default() };
	let discord =
		IdentityInfo { display: raw(b"proposer"), discord: handle, ..Default::default() };

	for info in [telegram, discord] {
		new_test_ext().execute_with(|| {
			let who = funded(Sr25519Keyring::Alice);
			judged_identity(&who, Judgement::Reasonable, info.clone());

			assert_ok!(submit(&who));
			assert_eq!(referendum_count(), 1);
		});
	}
}

#[test]
fn sub_of_qualified_identity_submits() {
	new_test_ext().execute_with(|| {
		let parent = funded(Sr25519Keyring::Alice);
		judged_identity(&parent, Judgement::Reasonable, identity_info(raw(b"@proposer")));
		let sub = funded(Sr25519Keyring::Bob);
		assert_ok!(Identity::set_subs(
			RuntimeOrigin::signed(parent),
			vec![(sub.clone(), Data::None)],
		));

		assert_ok!(submit(&sub));
		assert_eq!(referendum_count(), 1);
	});
}

#[test]
fn sub_of_unqualified_identity_cannot_submit() {
	new_test_ext().execute_with(|| {
		let parent = funded(Sr25519Keyring::Alice);
		assert_ok!(Identity::set_identity(
			RuntimeOrigin::signed(parent.clone()),
			Box::new(identity_info(raw(b"@proposer"))),
		));
		let sub = funded(Sr25519Keyring::Bob);
		assert_ok!(Identity::set_subs(
			RuntimeOrigin::signed(parent),
			vec![(sub.clone(), Data::None)],
		));

		assert_noop!(submit(&sub), DispatchError::BadOrigin);
		assert_eq!(referendum_count(), 0);
	});
}
