//! Validator entry identity gate wiring. Candidates joining the validator
//! set are held to the same qualified identity standard as referenda
//! submission, so a judgement without a social channel is not enough.

mod common;

use common::new_test_ext;
use frame_support::{
	assert_ok,
	traits::{tokens::fungible::Mutate, Contains},
};
use numen_runtime::{
	identity_info::IdentityInfo, AccountId, Balance, Balances, Identity, Runtime, RuntimeOrigin,
	UNIT,
};
use pallet_identity::{Data, Judgement};
use sp_keyring::Sr25519Keyring;
use sp_runtime::traits::{Hash, StaticLookup};

const FUNDS: Balance = 10_000 * UNIT;

type Gate = <Runtime as pallet_validator::Config>::IdentityGate;
type IdInfo = <Runtime as pallet_identity::Config>::IdentityInformation;

fn src(who: &AccountId) -> <<Runtime as frame_system::Config>::Lookup as StaticLookup>::Source {
	<Runtime as frame_system::Config>::Lookup::unlookup(who.clone())
}

fn raw(bytes: &[u8]) -> Data {
	Data::Raw(bytes.to_vec().try_into().expect("raw data fits the field bound"))
}

fn identity_info(x: Data) -> IdInfo {
	IdentityInfo { display: raw(b"candidate"), x, ..Default::default() }
}

/// Registers `info` for `who` and judges it Reasonable through a registrar
/// installed via the prime key.
fn judged_identity(who: &AccountId, info: IdInfo) {
	let prime = Sr25519Keyring::Ferdie.to_account_id();
	pallet_prime::Key::<Runtime>::put(&prime);
	let registrar = Sr25519Keyring::Eve.to_account_id();
	assert_ok!(Identity::add_registrar(
		RuntimeOrigin::signed(prime),
		src(&registrar),
	));
	Balances::set_balance(who, FUNDS);
	assert_ok!(Identity::set_identity(
		RuntimeOrigin::signed(who.clone()),
		Box::new(info.clone()),
	));
	assert_ok!(Identity::provide_judgement(
		RuntimeOrigin::signed(registrar),
		0,
		src(who),
		Judgement::Reasonable,
		<Runtime as frame_system::Config>::Hashing::hash_of(&info),
	));
}

#[test]
fn judged_identity_without_channel_fails_the_gate() {
	new_test_ext().execute_with(|| {
		let who = Sr25519Keyring::Alice.to_account_id();
		judged_identity(&who, identity_info(Data::None));

		assert!(!Gate::contains(&who));
	});
}

#[test]
fn qualified_identity_passes_the_gate() {
	new_test_ext().execute_with(|| {
		let who = Sr25519Keyring::Alice.to_account_id();
		judged_identity(&who, identity_info(raw(b"@candidate")));

		assert!(Gate::contains(&who));
	});
}
