//! Identity wiring. Registrar and username entry points answer only to the
//! prime key, deposits are priced per encoded byte, and nobody can force an
//! identity off the chain.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{assert_noop, assert_ok, traits::tokens::fungible::Mutate};
use numen_runtime::{
	identity_info::{IdentityField, IdentityInfo},
	AccountId, Balance, Balances, Identity, Runtime, RuntimeOrigin,
	UNIT,
};
use pallet_identity::IdentityInformationProvider;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{traits::StaticLookup, DispatchError};

const FUNDS: Balance = 10_000 * UNIT;

type IdInfo = <Runtime as pallet_identity::Config>::IdentityInformation;

fn install_prime() -> AccountId {
	let key = Sr25519Keyring::Ferdie.to_account_id();
	pallet_prime::Key::<Runtime>::put(&key);
	key
}

fn src(who: &AccountId) -> <<Runtime as frame_system::Config>::Lookup as StaticLookup>::Source {
	<Runtime as frame_system::Config>::Lookup::unlookup(who.clone())
}

/// An identity carrying a display name so the encoded size exceeds the empty
/// baseline, exercising the per-byte deposit.
fn identity_info() -> IdInfo {
	IdentityInfo {
		display: b"numen-dev".to_vec().try_into().unwrap(),
		..Default::default()
	}
}

fn expected_deposit(info: &IdInfo) -> Balance {
	let basic = <Runtime as pallet_identity::Config>::BasicDeposit::get();
	let byte = <Runtime as pallet_identity::Config>::ByteDeposit::get();
	basic + byte * info.encoded_size() as Balance
}

#[test]
fn add_registrar_accepts_prime_rejects_others() {
	new_test_ext().execute_with(|| {
		let key = install_prime();
		let stranger = Sr25519Keyring::Alice.to_account_id();

		assert_noop!(
			Identity::add_registrar(RuntimeOrigin::signed(stranger.clone()), src(&stranger)),
			DispatchError::BadOrigin,
		);
		assert_noop!(
			Identity::add_registrar(RuntimeOrigin::root(), src(&stranger)),
			DispatchError::BadOrigin,
		);
		assert!(pallet_identity::Registrars::<Runtime>::get().is_empty());

		assert_ok!(Identity::add_registrar(RuntimeOrigin::signed(key), src(&stranger)));
		assert_eq!(pallet_identity::Registrars::<Runtime>::get().len(), 1);
	});
}

#[test]
fn identity_deposit_prices_encoded_bytes() {
	new_test_ext().execute_with(|| {
		let who = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&who, FUNDS);
		let info = identity_info();

		assert_ok!(Identity::set_identity(
			RuntimeOrigin::signed(who.clone()),
			Box::new(info.clone()),
		));

		assert_eq!(Balances::reserved_balance(&who), expected_deposit(&info));
	});
}

/// Pins the decision to drop force removal. Re-wiring `ForceOrigin` back to
/// prime would let it slash any account's identity deposit.
#[test]
fn prime_cannot_kill_identity() {
	new_test_ext().execute_with(|| {
		let key = install_prime();
		let who = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&who, FUNDS);
		let info = identity_info();
		assert_ok!(Identity::set_identity(
			RuntimeOrigin::signed(who.clone()),
			Box::new(info.clone()),
		));
		let deposit = expected_deposit(&info);

		assert_noop!(
			Identity::kill_identity(RuntimeOrigin::signed(key), src(&who)),
			DispatchError::BadOrigin,
		);

		assert_eq!(Balances::reserved_balance(&who), deposit);
	});
}

/// The bit order the registrar service and the wallet both read fields by. A
/// registrar declares what it checks as a mask over these, so reordering the
/// struct silently retargets every declaration already on chain.
#[test]
fn identity_fields_keep_their_bits() {
	assert_eq!(IdentityField::Display as u64, 1 << 0);
	assert_eq!(IdentityField::Avatar as u64, 1 << 1);
	assert_eq!(IdentityField::About as u64, 1 << 2);
	assert_eq!(IdentityField::Web as u64, 1 << 3);
	assert_eq!(IdentityField::Email as u64, 1 << 4);
	assert_eq!(IdentityField::Github as u64, 1 << 5);
	assert_eq!(IdentityField::Matrix as u64, 1 << 6);
	assert_eq!(IdentityField::X as u64, 1 << 7);
	assert_eq!(IdentityField::Telegram as u64, 1 << 8);
	assert_eq!(IdentityField::Discord as u64, 1 << 9);
}

/// What `has_identity` answers, which is how a registrar checks a record
/// against the declaration it made.
#[test]
fn a_filled_field_shows_up_in_has_identity() {
	let telegram = IdentityField::Telegram as u64;
	let discord = IdentityField::Discord as u64;
	let info = IdentityInfo {
		telegram: b"numen".to_vec().try_into().unwrap(),
		..identity_info()
	};

	assert!(info.has_identity(telegram));
	assert!(!info.has_identity(telegram | discord));
	assert!(!IdentityInfo::default().has_identity(telegram));
}
