//! Constants the runtime publishes for off chain callers.
//!
//! Each one restates a value that the static metadata cannot otherwise reach.
//! A published copy is only worth having while it still agrees with the thing
//! it speaks for, so each gets pinned to its source here.

use codec::Decode;
use frame_support::{
	__private::metadata::{RuntimeMetadata, RuntimeMetadataPrefixed},
	traits::EnsureOrigin,
};
use numen_runtime::{
	configs::governance::{pallet_custom_origins, TracksInfo, TreasurySpender},
	Balance, Runtime, RuntimeOrigin,
};
use pallet_referenda::TracksInfo as _;

/// SCALE payload the metadata carries for one published constant.
fn published(pallet: &str, constant: &str) -> Vec<u8> {
	let RuntimeMetadataPrefixed(_, metadata) = Runtime::metadata();
	let RuntimeMetadata::V14(v14) = metadata else {
		panic!("construct_runtime hands back V14");
	};
	v14.pallets
		.iter()
		.find(|p| p.name == pallet)
		.unwrap_or_else(|| panic!("{pallet} is in the runtime"))
		.constants
		.iter()
		.find(|c| c.name == constant)
		.unwrap_or_else(|| panic!("{pallet} publishes {constant}"))
		.value
		.clone()
}

fn spend_caps() -> Vec<(u16, pallet_custom_origins::Origin, Balance)> {
	Decode::decode(&mut &published("Origins", "SpendCaps")[..])
		.expect("SpendCaps is published as a track keyed list of origins and ceilings")
}

/// The caps exist so a caller can size a proposal without submitting it. That
/// only holds while the published figure is the figure the origin hands out.
#[test]
fn every_published_cap_is_what_its_spender_origin_releases() {
	let caps = spend_caps();
	assert!(!caps.is_empty(), "the runtime publishes at least one spender track");

	for (id, origin, cap) in caps {
		let Ok(released) = TreasurySpender::try_origin(RuntimeOrigin::from(origin)) else {
			panic!("the origin published for track {id} clears TreasurySpender");
		};

		assert_eq!(cap, released, "track {id}");
	}
}

/// A track with no published cap leaves a caller guessing, and a cap with no
/// track points at a referendum nobody can open. Adding one without the other
/// is the drift this pairing exists to catch.
#[test]
fn published_caps_and_referendum_tracks_describe_the_same_set() {
	let capped: Vec<u16> = spend_caps().into_iter().map(|(id, _, _)| id).collect();
	let tracks: Vec<u16> = TracksInfo::tracks().map(|track| track.id).collect();

	assert_eq!(capped, tracks);
}

/// An external miner reads this to decide whether it speaks the protocol this
/// chain verifies. A stale copy sends it off hashing work no block will take.
#[test]
fn published_protocol_is_the_one_the_runtime_verifies_against() {
	let protocol: Vec<u8> = Decode::decode(&mut &published("Poscan", "Protocol")[..])
		.expect("Protocol is published as bytes");

	assert_eq!(protocol, poscan::POSCAN_PROTOCOL);
}

/// An indexer picks the PoW seal out of a block's digest logs by this tag, so a
/// wrong copy makes every block look like it carries no seal at all.
#[test]
fn published_engine_is_the_one_the_digest_carries() {
	let engine: [u8; 4] = Decode::decode(&mut &published("Poscan", "Engine")[..])
		.expect("Engine is published as four bytes");

	assert_eq!(engine, sp_consensus_pow::POW_ENGINE_ID);
}
