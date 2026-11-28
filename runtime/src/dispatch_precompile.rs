//! Precompile that runs an allowed `RuntimeCall` on behalf of the EOA caller.

use alloc::vec::Vec;
use codec::DecodeLimit;
use core::num::NonZero;
use frame_support::{
	dispatch::{DispatchClass, DispatchResultWithPostInfo, GetDispatchInfo, Pays},
	storage::with_transaction,
	weights::Weight,
};
use pallet_revive::precompiles::{
	alloy::{sol, sol_types::Revert},
	AddressMatcher, Error, Ext, Precompile, RuntimeCosts, H160,
};
use sp_runtime::{traits::Dispatchable, TransactionOutcome};

use crate::{Runtime, RuntimeCall, RuntimeOrigin};

const SLOT: u16 = 0x0803;

/// Max nesting depth accepted when decoding a SCALE call payload.
pub const DECODE_DEPTH_LIMIT: u32 = 8;

/// Address where pallet-revive mounts this precompile.
pub const ADDRESS: H160 =
	H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, (SLOT >> 8) as u8, SLOT as u8, 0, 0]);

const _: () = assert!(
	matches!(DispatchPrecompile::MATCHER.base_address(), b if b[16] == 0x08 && b[17] == 0x03 && b[18] == 0 && b[19] == 0)
);

sol! {
	interface IDispatch {
		function dispatch(bytes calldata call) external;
	}
}

use IDispatch::IDispatchCalls as Call;

pub struct DispatchPrecompile;

impl Precompile for DispatchPrecompile {
	type T = Runtime;
	type Interface = Call;
	const MATCHER: AddressMatcher = AddressMatcher::Fixed(NonZero::new(SLOT).unwrap());
	const HAS_CONTRACT_INFO: bool = false;

	fn call(
		_address: &[u8; 20],
		input: &Call,
		env: &mut impl Ext<T = Runtime>,
	) -> Result<Vec<u8>, Error> {
		if env.is_read_only() {
			return Err(pallet_revive::Error::<Runtime>::StateChangeDenied.into());
		}
		if env.is_delegate_call() {
			return Err(pallet_revive::Error::<Runtime>::PrecompileDelegateDenied.into());
		}
		env.frame_meter_mut().charge_weight_token(RuntimeCosts::CallerIsOrigin)?;
		if !env.caller_is_origin(false) {
			return Err(revert("Contract callers not allowed"));
		}
		if !env.value_transferred().is_zero() {
			return Err(revert("Function is not payable"));
		}

		let Call::dispatch(call) = input;
		let decode_weight = Weight::from_parts((call.call.len() as u64).saturating_mul(1_000), 0);
		env.charge(decode_weight)?;

		let runtime_call = RuntimeCall::decode_all_with_depth_limit(
			DECODE_DEPTH_LIMIT,
			&mut &call.call[..],
		)
		.map_err(|_| revert("Invalid runtime call"))?;

		if !is_allowed(&runtime_call) {
			return Err(revert("Call not allowed"));
		}

		let info = runtime_call.get_dispatch_info();
		if info.pays_fee != Pays::Yes || info.class != DispatchClass::Normal {
			return Err(revert("Call not allowed"));
		}

		let charged = env.charge(info.total_weight())?;
		env.frame_meter_mut().charge_weight_token(RuntimeCosts::Caller)?;
		let who = env
			.caller()
			.account_id()
			.map_err(|_| revert("Caller has no account"))?
			.clone();

		let outcome = with_transaction(|| -> TransactionOutcome<DispatchResultWithPostInfo> {
			match runtime_call.dispatch(RuntimeOrigin::signed(who)) {
				Ok(post_info) => TransactionOutcome::Commit(Ok(post_info)),
				Err(err) => TransactionOutcome::Rollback(Err(err)),
			}
		});

		match outcome {
			Ok(post_info) => {
				if let Some(actual) = post_info.actual_weight {
					env.adjust_gas(charged, actual);
				}
				Ok(Vec::new())
			},
			Err(err) => {
				if let Some(actual) = err.post_info.actual_weight {
					env.adjust_gas(charged, actual);
				}
				Err(revert(<&'static str>::from(err.error)))
			},
		}
	}
}

/// Calls whose indexer and service invariants hold when run inside `eth_transact`.
pub fn is_allowed(call: &RuntimeCall) -> bool {
	match call {
		RuntimeCall::ConvictionVoting(_) | RuntimeCall::Vesting(_) => true,
		RuntimeCall::Referenda(
			pallet_referenda::Call::place_decision_deposit { .. }
			| pallet_referenda::Call::refund_decision_deposit { .. }
			| pallet_referenda::Call::refund_submission_deposit { .. },
		) => true,
		RuntimeCall::Treasury(pallet_treasury::Call::payout { .. }) => true,
		RuntimeCall::Utility(
			pallet_utility::Call::batch { calls }
			| pallet_utility::Call::batch_all { calls }
			| pallet_utility::Call::force_batch { calls },
		) => calls.iter().all(is_allowed),
		_ => false,
	}
}

fn revert(reason: &str) -> Error {
	Error::Revert(Revert { reason: reason.into() })
}
