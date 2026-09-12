//! OpenGov wiring. Tiered spender origins approve treasury backed bounties up
//! to each tier cap, and a higher tier clears what a lower tier cannot. Root
//! calls and runtime code reach the ballot on tracks of their own.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::{
	assert_noop, assert_ok,
	traits::{
		schedule::DispatchTime, tokens::fungible::Mutate, OnInitialize, OriginTrait, StorePreimage,
	},
};
use pallet_bounties::BountyStatus;
use pallet_conviction_voting::{AccountVote, Conviction, Vote};
use pallet_referenda::{ReferendumInfo, ReferendumInfoFor};
use numen_runtime::{
	configs::governance::{pallet_custom_origins, TracksInfo},
	AccountId, Balance, Balances, BlockNumber, Bounties, ConvictionVoting, Preimage, Prime,
	Referenda, Runtime, RuntimeCall, RuntimeOrigin, Scheduler, System, UNIT, VERSION,
};
use sp_core::traits::{Externalities, ReadRuntimeVersion, ReadRuntimeVersionExt};
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use sp_runtime::DispatchError;
use sp_version::RuntimeVersion;

/// Small tier funding cap. A referendum on the small track releases at most this.
const SMALL_CAP: Balance = pallet_custom_origins::SMALL_SPENDER_CAP;

/// Bankroll for the account that submits and votes. Small track support is
/// weighed against active issuance, so its aye capital must dominate the supply.
const VOTER_FUNDS: Balance = 1_000_000 * UNIT;

/// Aye capital the voter commits, a clear supermajority of the supply.
const AYE_CAPITAL: Balance = 900_000 * UNIT;

/// Fund a proposer and place a bounty of `value`, returning its id.
fn proposed_bounty(value: Balance) -> u32 {
	let proposer = Sr25519Keyring::Alice.to_account_id();
	Balances::set_balance(&proposer, 10_000 * UNIT);
	let id = pallet_bounties::BountyCount::<Runtime>::get();
	assert_ok!(Bounties::propose_bounty(
		RuntimeOrigin::signed(proposer),
		value,
		b"work".to_vec(),
	));
	id
}

fn assert_queued_for_funding(id: u32) {
	let bounty = pallet_bounties::Bounties::<Runtime>::get(id).expect("bounty exists");
	assert!(matches!(bounty.get_status(), BountyStatus::Approved));
	assert!(
		pallet_bounties::BountyApprovals::<Runtime>::get().contains(&id),
		"an approved bounty must be queued for the next spend period",
	);
}

fn assert_still_proposed(id: u32) {
	let bounty = pallet_bounties::Bounties::<Runtime>::get(id).expect("bounty exists");
	assert!(
		matches!(bounty.get_status(), BountyStatus::Proposed),
		"a rejected approval must leave the bounty proposed",
	);
	assert!(pallet_bounties::BountyApprovals::<Runtime>::get().is_empty());
}

#[test]
fn small_spender_approves_bounty_at_its_cap() {
	new_test_ext().execute_with(|| {
		let id = proposed_bounty(SMALL_CAP);
		let small = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);

		assert_ok!(Bounties::approve_bounty(small, id));

		assert_queued_for_funding(id);
	});
}

#[test]
fn small_spender_cannot_approve_above_its_cap() {
	new_test_ext().execute_with(|| {
		let id = proposed_bounty(SMALL_CAP + 1);
		let small = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);

		assert_noop!(
			Bounties::approve_bounty(small, id),
			pallet_treasury::Error::<Runtime>::InsufficientPermission,
		);

		assert_still_proposed(id);
	});
}

#[test]
fn higher_tier_approves_what_lower_tier_cannot() {
	new_test_ext().execute_with(|| {
		let id = proposed_bounty(SMALL_CAP + 1);
		let medium = RuntimeOrigin::from(pallet_custom_origins::Origin::MediumSpender);

		assert_ok!(Bounties::approve_bounty(medium, id));

		assert_queued_for_funding(id);
	});
}

#[test]
fn root_origin_cannot_approve_bounty() {
	new_test_ext().execute_with(|| {
		let id = proposed_bounty(SMALL_CAP + 1);

		assert_noop!(
			Bounties::approve_bounty(RuntimeOrigin::root(), id),
			DispatchError::BadOrigin,
		);

		assert_still_proposed(id);
	});
}

#[test]
fn signed_origin_cannot_approve_bounty() {
	new_test_ext().execute_with(|| {
		let id = proposed_bounty(1_000 * UNIT);
		let who = Sr25519Keyring::Bob.to_account_id();

		assert_noop!(
			Bounties::approve_bounty(RuntimeOrigin::signed(who), id),
			DispatchError::BadOrigin,
		);

		assert_still_proposed(id);
	});
}

#[test]
fn identity_admin_dispatches_on_its_own_track() {
	let admin = RuntimeOrigin::from(pallet_custom_origins::Origin::IdentityAdmin);
	let id = <TracksInfo as pallet_referenda::TracksInfo<Balance, BlockNumber>>::track_for(
		admin.caller(),
	)
	.expect("the identity admin origin has a track");

	assert_eq!(id, 10);
	assert!(<TracksInfo as pallet_referenda::TracksInfo<Balance, BlockNumber>>::info(id).is_some());
}

/// Small track prepare, confirm and enactment periods, read from the live track
/// so the walk below follows whatever the runtime configures.
fn small_track_periods() -> (BlockNumber, BlockNumber, BlockNumber) {
	let small = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);
	let id = <TracksInfo as pallet_referenda::TracksInfo<Balance, BlockNumber>>::track_for(
		small.caller(),
	)
	.expect("the small spender origin has a track");
	let info = <TracksInfo as pallet_referenda::TracksInfo<Balance, BlockNumber>>::info(id)
		.expect("small track exists");
	(info.prepare_period, info.confirm_period, info.min_enactment_period)
}

/// Advance to `n`, servicing scheduler agendas each block so referendum alarms
/// and the scheduled enactment call fire on their target blocks.
fn run_to_block(n: BlockNumber) {
	while System::block_number() < n {
		let next = System::block_number() + 1;
		System::set_block_number(next);
		Scheduler::on_initialize(next);
	}
}

/// Submit a small track referendum carrying `call`, place its decision deposit
/// and back it with a supermajority, returning its index. The tally clears both
/// small track bars at the first deciding block.
/// Submission is gated on a qualified identity, which has its own coverage, so
/// place a judged one carrying a plaintext channel straight into storage.
fn qualify(who: &AccountId) {
	pallet_identity::IdentityOf::<Runtime>::insert(
		who,
		pallet_identity::Registration {
			judgements: vec![(0, pallet_identity::Judgement::Reasonable)]
				.try_into()
				.expect("one judgement fits the bound"),
			deposit: 0,
			info: numen_runtime::identity_info::IdentityInfo {
				x: b"voter".to_vec().try_into().expect("the handle fits the field bound"),
				..Default::default()
			},
		},
	);
}

fn backed_small_track_referendum(voter: &AccountId, call: RuntimeCall) -> u32 {
	Balances::set_balance(voter, VOTER_FUNDS);
	qualify(voter);
	let index = pallet_referenda::ReferendumCount::<Runtime>::get();
	let proposal = <Preimage as StorePreimage>::bound(call).expect("the call bounds inline");
	let track = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);
	assert_ok!(Referenda::submit(
		RuntimeOrigin::signed(voter.clone()),
		Box::new(track.caller().clone()),
		proposal,
		DispatchTime::After(1),
	));
	assert_ok!(Referenda::place_decision_deposit(RuntimeOrigin::signed(voter.clone()), index));
	assert_ok!(ConvictionVoting::vote(
		RuntimeOrigin::signed(voter.clone()),
		index,
		AccountVote::Standard {
			vote: Vote { aye: true, conviction: Conviction::Locked1x },
			balance: AYE_CAPITAL,
		},
	));
	index
}

#[test]
fn spend_referendum_runs_from_submission_through_confirm_to_dispatch() {
	new_test_ext().execute_with(|| {
		let voter = Sr25519Keyring::Bob.to_account_id();
		let bounty = proposed_bounty(1_000 * UNIT);
		let index = backed_small_track_referendum(
			&voter,
			RuntimeCall::Bounties(pallet_bounties::Call::approve_bounty { bounty_id: bounty }),
		);

		let (prepare, confirm, enact) = small_track_periods();
		let confirmed_at = System::block_number() + prepare + confirm;
		let enacted_at = confirmed_at + enact;

		// Just submitted, the poll is live and the spend has not touched the bounty.
		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Ongoing(_)),
		));
		assert_still_proposed(bounty);

		// Prepare then confirm elapse, the poll passes yet the spend still waits
		// out the enactment delay before it can run.
		run_to_block(confirmed_at);
		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Approved(..)),
		));
		assert_still_proposed(bounty);

		// Enactment dispatches the call under the small spender origin the poll
		// minted, and only that origin can flip the bounty it targets.
		run_to_block(enacted_at + 1);
		assert_queued_for_funding(bounty);
	});
}

/// Root dispatches the calls a chain most needs to put to its holders, so it
/// carries a track of its own.
#[test]
fn a_root_call_reaches_the_ballot() {
	new_test_ext().execute_with(|| {
		let submitter = Sr25519Keyring::Bob.to_account_id();
		Balances::set_balance(&submitter, VOTER_FUNDS);
		qualify(&submitter);
		let index = pallet_referenda::ReferendumCount::<Runtime>::get();
		let proposal = <Preimage as StorePreimage>::bound(RuntimeCall::System(
			frame_system::Call::remark { remark: b"root".to_vec() },
		))
		.expect("a remark call bounds inline");

		assert_ok!(Referenda::submit(
			RuntimeOrigin::signed(submitter),
			Box::new(RuntimeOrigin::root().caller().clone()),
			proposal,
			DispatchTime::After(1),
		));

		assert!(matches!(
			ReferendumInfoFor::<Runtime>::get(index),
			Some(ReferendumInfo::Ongoing(_)),
		));
	});
}

/// Version probe stub fed to the externalities in place of a wasm executor.
struct VersionStub(Vec<u8>);

impl ReadRuntimeVersion for VersionStub {
	fn read_runtime_version(
		&self,
		_wasm_code: &[u8],
		_ext: &mut dyn Externalities,
	) -> Result<Vec<u8>, String> {
		Ok(self.0.clone())
	}
}

/// Externalities whose version probe reports a runtime one spec version ahead,
/// which is what `set_code` demands.
fn ext_accepting_an_upgrade() -> TestExternalities {
	let next = RuntimeVersion { spec_version: VERSION.spec_version + 1, ..VERSION };
	let mut ext = new_test_ext();
	ext.register_extension(ReadRuntimeVersionExt::new(VersionStub(next.encode())));
	ext
}

/// The upgrade track is only worth having while the origin it mints is one
/// that `upgrade` accepts.
#[test]
fn the_upgrade_track_replaces_runtime_code() {
	ext_accepting_an_upgrade().execute_with(|| {
		let track = RuntimeOrigin::from(pallet_custom_origins::Origin::RuntimeUpgrade);

		assert_ok!(Prime::upgrade(track, b"a new runtime".to_vec()));

		System::assert_has_event(frame_system::Event::CodeUpdated.into());
	});
}

/// Every other track mints an origin of its own, and none of them reaches the
/// runtime code.
#[test]
fn no_other_track_replaces_runtime_code() {
	let others = [
		pallet_custom_origins::Origin::WishForChange,
		pallet_custom_origins::Origin::IdentityAdmin,
		pallet_custom_origins::Origin::ReferendumCanceller,
		pallet_custom_origins::Origin::ReferendumKiller,
		pallet_custom_origins::Origin::SmallSpender,
		pallet_custom_origins::Origin::MediumSpender,
		pallet_custom_origins::Origin::BigSpender,
	];

	ext_accepting_an_upgrade().execute_with(|| {
		for origin in others {
			assert_noop!(
				Prime::upgrade(RuntimeOrigin::from(origin), b"a new runtime".to_vec()),
				DispatchError::BadOrigin,
			);
		}
	});
}

/// The prime key holds the brakes, not the code.
#[test]
fn the_prime_key_cannot_replace_runtime_code() {
	ext_accepting_an_upgrade().execute_with(|| {
		let key = Sr25519Keyring::Ferdie.to_account_id();
		pallet_prime::Key::<Runtime>::put(&key);

		assert_noop!(
			Prime::upgrade(RuntimeOrigin::signed(key), b"a new runtime".to_vec()),
			DispatchError::BadOrigin,
		);
	});
}
