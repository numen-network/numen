use crate::{mock::*, EnsurePrime, Error, Event, Key};
use frame_support::{assert_noop, assert_ok, traits::EnsureOrigin};
use pallet_identity::{Judgement, RegistrarIndex};
use sp_runtime::{traits::Hash, BuildStorage, DispatchError};
use sp_version::RuntimeVersion;

type Info = <Test as pallet_identity::Config>::IdentityInformation;

fn seat_registrar() -> RegistrarIndex {
	assert_ok!(Identity::add_registrar(RuntimeOrigin::root(), REGISTRAR));
	pallet_identity::Registrars::<Test>::get().len() as RegistrarIndex - 1
}

fn register_subject() -> <Test as frame_system::Config>::Hash {
	let info = Info::default();
	let info_hash = <Test as frame_system::Config>::Hashing::hash_of(&info);
	assert_ok!(Identity::set_identity(
		RuntimeOrigin::signed(SUBJECT),
		Box::new(info)
	));
	info_hash
}

fn upgrade_version(spec_version: u32) -> RuntimeVersion {
	RuntimeVersion {
		spec_name: "test".into(),
		spec_version,
		..Default::default()
	}
}

#[test]
fn genesis_key_lands_in_storage() {
	new_test_ext().execute_with(|| {
		assert_eq!(Key::<Test>::get(), Some(PRIME));
	});
}

#[test]
fn empty_key_rejects_everyone() {
	let t = frame_system::GenesisConfig::<Test>::default()
		.build_storage()
		.unwrap();
	sp_io::TestExternalities::from(t).execute_with(|| {
		assert_noop!(
			Prime::set_key(RuntimeOrigin::signed(PRIME), OTHER),
			Error::<Test>::RequirePrime,
		);
	});
}

#[test]
fn upgrade_replaces_runtime_code() {
	new_test_ext_with_version(upgrade_version(2)).execute_with(|| {
		assert_ok!(Prime::upgrade(RuntimeOrigin::signed(PRIME), vec![1, 2, 3]));
		System::assert_has_event(frame_system::Event::CodeUpdated.into());
	});
}

// `frame_system` compiles out the spec version check under
// `runtime-benchmarks` so that its own `set_code` benchmark can run, and the
// `not(test)` guard on that switch does not apply to a dependency crate.
#[test]
#[cfg(not(feature = "runtime-benchmarks"))]
fn upgrade_keeps_system_version_checks() {
	new_test_ext_with_version(upgrade_version(1)).execute_with(|| {
		assert_noop!(
			Prime::upgrade(RuntimeOrigin::signed(PRIME), vec![1, 2, 3]),
			frame_system::Error::<Test>::SpecVersionNeedsToIncrease,
		);
	});
}

#[test]
fn upgrade_rejects_non_prime() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Prime::upgrade(RuntimeOrigin::signed(OTHER), vec![1, 2, 3]),
			Error::<Test>::RequirePrime,
		);
	});
}

#[test]
fn upgrade_rejects_unsigned_origins() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Prime::upgrade(RuntimeOrigin::root(), vec![1, 2, 3]),
			DispatchError::BadOrigin,
		);
		assert_noop!(
			Prime::upgrade(RuntimeOrigin::none(), vec![1, 2, 3]),
			DispatchError::BadOrigin,
		);
	});
}

#[test]
fn set_key_rotates_key() {
	new_test_ext().execute_with(|| {
		assert_ok!(Prime::set_key(RuntimeOrigin::signed(PRIME), OTHER));
		assert_eq!(Key::<Test>::get(), Some(OTHER));
		System::assert_last_event(Event::<Test>::KeyChanged { old: PRIME, new: OTHER }.into());

		assert_noop!(
			Prime::set_key(RuntimeOrigin::signed(PRIME), PRIME),
			Error::<Test>::RequirePrime,
		);
		assert_ok!(Prime::set_key(RuntimeOrigin::signed(OTHER), PRIME));
	});
}

#[test]
fn set_key_rejects_non_prime() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Prime::set_key(RuntimeOrigin::signed(OTHER), OTHER),
			Error::<Test>::RequirePrime,
		);
	});
}

#[test]
fn ensure_prime_origin_accepts_prime_key() {
	new_test_ext().execute_with(|| {
		assert!(EnsurePrime::<Test>::try_origin(RuntimeOrigin::signed(PRIME)).is_ok());
	});
}

#[test]
fn ensure_prime_origin_rejects_others() {
	new_test_ext().execute_with(|| {
		assert!(EnsurePrime::<Test>::try_origin(RuntimeOrigin::signed(OTHER)).is_err());
		assert!(EnsurePrime::<Test>::try_origin(RuntimeOrigin::root()).is_err());
		assert!(EnsurePrime::<Test>::try_origin(RuntimeOrigin::none()).is_err());
	});
}

#[test]
fn ensure_prime_origin_rejects_everyone_without_key() {
	let t = frame_system::GenesisConfig::<Test>::default()
		.build_storage()
		.unwrap();
	sp_io::TestExternalities::from(t).execute_with(|| {
		assert!(EnsurePrime::<Test>::try_origin(RuntimeOrigin::signed(PRIME)).is_err());
	});
}

#[test]
fn remove_registrar_empties_the_seat() {
	new_test_ext().execute_with(|| {
		let index = seat_registrar();

		assert_ok!(Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index));

		assert_eq!(
			pallet_identity::Registrars::<Test>::get()[index as usize],
			None
		);
		System::assert_last_event(Event::<Test>::RegistrarRemoved { index }.into());
	});
}

#[test]
fn removed_registrar_cannot_judge() {
	new_test_ext().execute_with(|| {
		let index = seat_registrar();
		let info_hash = register_subject();

		assert_ok!(Identity::provide_judgement(
			RuntimeOrigin::signed(REGISTRAR),
			index,
			SUBJECT,
			Judgement::Reasonable,
			info_hash,
		));

		assert_ok!(Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index));

		assert_noop!(
			Identity::provide_judgement(
				RuntimeOrigin::signed(REGISTRAR),
				index,
				SUBJECT,
				Judgement::KnownGood,
				info_hash,
			),
			pallet_identity::Error::<Test>::InvalidIndex,
		);
	});
}

#[test]
fn removal_keeps_judgements_already_given() {
	new_test_ext().execute_with(|| {
		let index = seat_registrar();
		let info_hash = register_subject();
		assert_ok!(Identity::provide_judgement(
			RuntimeOrigin::signed(REGISTRAR),
			index,
			SUBJECT,
			Judgement::Reasonable,
			info_hash,
		));

		assert_ok!(Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index));

		assert_eq!(
			pallet_identity::IdentityOf::<Test>::get(SUBJECT)
				.expect("subject registered an identity")
				.judgements
				.into_inner(),
			vec![(index, Judgement::Reasonable)],
		);
	});
}

#[test]
fn remove_registrar_rejects_an_empty_seat() {
	new_test_ext().execute_with(|| {
		let index = seat_registrar();
		assert_ok!(Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index));

		assert_noop!(
			Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index),
			Error::<Test>::NoRegistrar,
		);
	});
}

#[test]
fn remove_registrar_rejects_an_index_past_the_last_seat() {
	new_test_ext().execute_with(|| {
		let index = seat_registrar();

		assert_noop!(
			Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index + 1),
			Error::<Test>::NoRegistrar,
		);
		assert_ok!(Prime::remove_registrar(RuntimeOrigin::signed(PRIME), index));
	});
}

#[test]
fn remove_registrar_rejects_non_prime() {
	new_test_ext().execute_with(|| {
		let index = seat_registrar();

		assert_noop!(
			Prime::remove_registrar(RuntimeOrigin::signed(OTHER), index),
			Error::<Test>::RequirePrime,
		);
	});
}
