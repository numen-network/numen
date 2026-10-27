//! Identity wiring. Registrars answer to the prime key and the identity admin
//! track, and a preset can seat them at genesis. Username authorities answer to
//! the track alone, only root can force an identity off the chain, and deposits
//! are priced per encoded byte.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{assert_noop, assert_ok, traits::tokens::fungible::Mutate};
use numen_runtime::{
	configs::governance::pallet_custom_origins,
	identity_info::{IdentityField, IdentityInfo},
	AccountId, Balance, Balances, Identity, Runtime, RuntimeOrigin, UNIT,
};
use numen_runtime::{genesis_config_presets as presets, RuntimeGenesisConfig};
use pallet_identity::{IdentityInformationProvider, Judgement};
use sp_core::crypto::Ss58Codec;
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	traits::{Hash, StaticLookup},
	DispatchError,
};

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

/// Chain state as a node would build it from `preset`.
fn preset_ext(preset: &str) -> TestExternalities {
	let mut config = serde_json::to_value(RuntimeGenesisConfig::default()).unwrap();
	let patch = presets::get_preset(&preset.into()).expect("preset exists");
	sc_chain_spec::json_merge(&mut config, serde_json::from_slice(&patch).unwrap());
	let mut ext = TestExternalities::default();
	ext.execute_with(|| {
		frame_support::genesis_builder_helper::build_state::<RuntimeGenesisConfig>(
			serde_json::to_vec(&config).unwrap(),
		)
		.expect("preset builds");
	});
	ext
}

#[test]
fn dev_presets_let_alice_judge_from_genesis() {
	for preset in [
		sp_genesis_builder::DEV_RUNTIME_PRESET,
		sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET,
		presets::INTEGRATION_RUNTIME_PRESET,
	] {
		preset_ext(preset).execute_with(|| {
			let alice = Sr25519Keyring::Alice.to_account_id();
			let bob = Sr25519Keyring::Bob.to_account_id();
			let info = identity_info();
			assert_ok!(Identity::set_identity(RuntimeOrigin::signed(bob.clone()), Box::new(info.clone())));

			assert_ok!(Identity::provide_judgement(
				RuntimeOrigin::signed(alice),
				0,
				src(&bob),
				Judgement::Reasonable,
				<Runtime as frame_system::Config>::Hashing::hash_of(&info),
			));

			let registration = pallet_identity::IdentityOf::<Runtime>::get(&bob).unwrap();
			assert_eq!(registration.judgements.into_inner(), vec![(0, Judgement::Reasonable)], "{preset}");
		});
	}
}

#[test]
fn testnet_preset_seats_the_registrar_bot_without_a_fee() {
	preset_ext(presets::TESTNET_RUNTIME_PRESET).execute_with(|| {
		let registrars = pallet_identity::Registrars::<Runtime>::get();
		assert_eq!(registrars.len(), 1);
		let bot = registrars[0].as_ref().unwrap();
		let (expected, _) =
			AccountId::from_ss58check_with_version("nu7iBot1ZuaFNt6wh4CQ8B8yh82Q6PsgJF7N7gfCGnoLLxZwv")
				.unwrap();
		assert_eq!(bot.account, expected);
		assert_eq!(bot.fee, 0);
	});
}

#[test]
fn mainnet_preset_writes_no_registrar_storage() {
	preset_ext(presets::MAINNET_RUNTIME_PRESET).execute_with(|| {
		assert!(!pallet_identity::Registrars::<Runtime>::exists());
	});
}

fn identity_admin() -> RuntimeOrigin {
	RuntimeOrigin::from(pallet_custom_origins::Origin::IdentityAdmin)
}

#[test]
fn identity_admin_track_seats_a_registrar() {
	new_test_ext().execute_with(|| {
		let stranger = Sr25519Keyring::Alice.to_account_id();

		assert_ok!(Identity::add_registrar(identity_admin(), src(&stranger)));

		assert_eq!(pallet_identity::Registrars::<Runtime>::get().len(), 1);
	});
}

#[test]
fn identity_admin_track_retires_a_registrar() {
	new_test_ext().execute_with(|| {
		let stranger = Sr25519Keyring::Alice.to_account_id();
		assert_ok!(Identity::add_registrar(identity_admin(), src(&stranger)));

		assert_ok!(Identity::remove_registrar(identity_admin(), 0));

		assert!(pallet_identity::Registrars::<Runtime>::get()[0].is_none());
	});
}

#[test]
fn identity_admin_track_appoints_and_retires_a_username_authority() {
	new_test_ext().execute_with(|| {
		let authority = Sr25519Keyring::Alice.to_account_id();

		assert_ok!(Identity::add_username_authority(
			identity_admin(),
			src(&authority),
			b"numen".to_vec(),
			10,
		));
		assert_eq!(pallet_identity::AuthorityOf::<Runtime>::iter().count(), 1);

		assert_ok!(Identity::remove_username_authority(
			identity_admin(),
			b"numen".to_vec(),
			src(&authority),
		));
		assert_eq!(pallet_identity::AuthorityOf::<Runtime>::iter().count(), 0);
	});
}

/// Naming an authority overwrites whoever holds the suffix, so the track keeps
/// both calls to itself.
#[test]
fn prime_cannot_touch_username_authorities() {
	new_test_ext().execute_with(|| {
		let key = install_prime();
		let authority = Sr25519Keyring::Alice.to_account_id();

		assert_noop!(
			Identity::add_username_authority(
				RuntimeOrigin::signed(key.clone()),
				src(&authority),
				b"numen".to_vec(),
				10,
			),
			DispatchError::BadOrigin,
		);

		assert_ok!(Identity::add_username_authority(
			identity_admin(),
			src(&authority),
			b"numen".to_vec(),
			10,
		));

		assert_noop!(
			Identity::remove_username_authority(
				RuntimeOrigin::signed(key),
				b"numen".to_vec(),
				src(&authority),
			),
			DispatchError::BadOrigin,
		);

		assert_eq!(pallet_identity::AuthorityOf::<Runtime>::iter().count(), 1);
	});
}

/// The track hands out seats and stops there. Forcing an identity off the
/// chain is root's.
#[test]
fn identity_admin_track_cannot_kill_identity() {
	new_test_ext().execute_with(|| {
		let who = Sr25519Keyring::Alice.to_account_id();
		Balances::set_balance(&who, FUNDS);
		assert_ok!(Identity::set_identity(
			RuntimeOrigin::signed(who.clone()),
			Box::new(identity_info()),
		));

		assert_noop!(
			Identity::kill_identity(identity_admin(), src(&who)),
			DispatchError::BadOrigin,
		);
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
	assert_eq!(IdentityField::Bio as u64, 1 << 2);
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
