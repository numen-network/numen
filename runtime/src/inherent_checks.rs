use codec::Decode;
use crate::{RuntimeCall, UncheckedExtrinsic};

/// Maximum allowed timestamp drift from the node's local clock (milliseconds).
pub const MAX_TIMESTAMP_DRIFT_MS: u64 = 2_000;

/// Reads the timestamp out of an encoded timestamp inherent, in milliseconds.
///
/// Anything else yields `None`, leaving the block with no clock for its difficulty.
pub fn timestamp_from_inherent(encoded: &[u8]) -> Option<u64> {
	let extrinsic = UncheckedExtrinsic::decode(&mut &encoded[..]).ok()?;
	match extrinsic.0.function {
		RuntimeCall::Timestamp(pallet_timestamp::Call::set { now }) => Some(now),
		_ => None,
	}
}

/// Validate block timestamp against drift limits.
///
/// Appends errors to `result` if the block timestamp exceeds the allowed
/// drift or is earlier than the parent timestamp.
pub fn check_timestamp_drift(
	result: &mut sp_inherents::CheckInherentsResult,
	block_ts_ms: u64,
	node_ts_ms: u64,
	parent_ts_ms: u64,
) {
	if block_ts_ms > node_ts_ms + MAX_TIMESTAMP_DRIFT_MS {
		let _ = result.put_error(
			sp_timestamp::INHERENT_IDENTIFIER,
			&sp_timestamp::InherentError::TooFarInFuture,
		);
	}

	if block_ts_ms < parent_ts_ms {
		let _ = result.put_error(
			sp_timestamp::INHERENT_IDENTIFIER,
			&sp_timestamp::InherentError::TooEarly,
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::Encode;
	use sp_inherents::CheckInherentsResult;
	use sp_runtime::generic;

	#[test]
	fn reads_a_timestamp_inherent() {
		let call = RuntimeCall::Timestamp(pallet_timestamp::Call::set { now: 123_000 });
		let extrinsic: UncheckedExtrinsic = generic::UncheckedExtrinsic::new_bare(call).into();
		assert_eq!(timestamp_from_inherent(&extrinsic.encode()), Some(123_000));
	}

	#[test]
	fn rejects_a_call_that_is_not_the_timestamp() {
		let call = RuntimeCall::System(frame_system::Call::remark { remark: Default::default() });
		let extrinsic: UncheckedExtrinsic = generic::UncheckedExtrinsic::new_bare(call).into();
		assert_eq!(timestamp_from_inherent(&extrinsic.encode()), None);
	}

	#[test]
	fn rejects_bytes_that_are_not_an_extrinsic() {
		assert_eq!(timestamp_from_inherent(&[0xff, 0xff, 0xff]), None);
	}

	fn run(block_ts_ms: u64, node_ts_ms: u64, parent_ts_ms: u64) -> CheckInherentsResult {
		let mut result = CheckInherentsResult::new();
		check_timestamp_drift(&mut result, block_ts_ms, node_ts_ms, parent_ts_ms);
		result
	}

	#[test]
	fn within_drift_accepted() {
		let result = run(120_000 + MAX_TIMESTAMP_DRIFT_MS, 120_000, 100_000);
		assert!(result.ok(), "should be accepted");
	}

	#[test]
	fn beyond_drift_rejected() {
		let result = run(120_000 + MAX_TIMESTAMP_DRIFT_MS + 1, 120_000, 100_000);
		assert!(!result.ok(), "should be rejected");
	}

	#[test]
	fn before_parent_rejected() {
		let result = run(100_000 - 1, 120_000, 100_000);
		assert!(!result.ok(), "should be rejected");
	}

	#[test]
	fn equal_to_parent_accepted() {
		let result = run(100_000, 120_000, 100_000);
		assert!(result.ok(), "should be accepted");
	}

}