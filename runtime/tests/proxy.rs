//! ProxyType filter wiring. NonTransfer fences off every fund moving entry
//! including contract calls, Governance admits only the governance pallets, and
//! pallet-proxy enforces the filter at dispatch.

mod common;

use common::new_test_ext;
use frame_support::{
	assert_ok,
	traits::{tokens::fungible::Mutate, InstanceFilter},
};
use numen_runtime::{
	configs::ProxyType, AccountId, Balances, Proxy, Runtime, RuntimeCall, RuntimeOrigin, System,
	UNIT,
};
use sp_keyring::Sr25519Keyring;
use sp_runtime::traits::StaticLookup;

fn src(who: &AccountId) -> <<Runtime as frame_system::Config>::Lookup as StaticLookup>::Source {
	<Runtime as frame_system::Config>::Lookup::unlookup(who.clone())
}

fn transfer_call(dest: &AccountId) -> RuntimeCall {
	RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive {
		dest: src(dest),
		value: UNIT,
	})
}

fn contract_call() -> RuntimeCall {
	RuntimeCall::Revive(pallet_revive::Call::call {
		dest: pallet_revive::H160::zero(),
		value: UNIT,
		weight_limit: frame_support::weights::Weight::zero(),
		storage_deposit_limit: 0,
		data: Vec::new(),
	})
}

fn remark_call() -> RuntimeCall {
	RuntimeCall::System(frame_system::Call::remark { remark: Vec::new() })
}

fn vested_transfer_call(dest: &AccountId) -> RuntimeCall {
	RuntimeCall::Vesting(pallet_vesting::Call::vested_transfer {
		target: src(dest),
		schedule: pallet_vesting::VestingInfo::new(UNIT, UNIT, 0),
	})
}

fn vest_calls() -> Vec<RuntimeCall> {
	vec![
		RuntimeCall::Vesting(pallet_vesting::Call::vest {}),
		RuntimeCall::Vesting(pallet_vesting::Call::vest_other {
			target: src(&Sr25519Keyring::Bob.to_account_id()),
		}),
		RuntimeCall::Vesting(pallet_vesting::Call::merge_schedules {
			schedule1_index: 0,
			schedule2_index: 1,
		}),
	]
}

fn governance_whitelist(voter: &AccountId) -> Vec<RuntimeCall> {
	vec![
		RuntimeCall::Treasury(pallet_treasury::Call::remove_approval { proposal_id: 0 }),
		RuntimeCall::Bounties(pallet_bounties::Call::propose_bounty {
			value: UNIT,
			description: Vec::new(),
		}),
		RuntimeCall::ChildBounties(pallet_child_bounties::Call::add_child_bounty {
			parent_bounty_id: 0,
			value: UNIT,
			description: Vec::new(),
		}),
		RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::unlock {
			class: 0,
			target: src(voter),
		}),
		RuntimeCall::Referenda(pallet_referenda::Call::place_decision_deposit { index: 0 }),
		RuntimeCall::Utility(pallet_utility::Call::batch { calls: Vec::new() }),
	]
}

#[test]
fn non_transfer_blocks_every_fund_moving_entry() {
	let dest = Sr25519Keyring::Bob.to_account_id();
	for call in [transfer_call(&dest), contract_call(), vested_transfer_call(&dest)] {
		assert!(
			!ProxyType::NonTransfer.filter(&call),
			"NonTransfer must block {call:?}",
		);
	}
}

#[test]
fn non_transfer_admits_calls_that_leave_funds_alone() {
	let voter = Sr25519Keyring::Bob.to_account_id();
	let admitted = governance_whitelist(&voter)
		.into_iter()
		.chain(vest_calls())
		.chain([remark_call()]);
	for call in admitted {
		assert!(
			ProxyType::NonTransfer.filter(&call),
			"NonTransfer must admit {call:?}",
		);
	}
}

#[test]
fn governance_admits_only_the_governance_whitelist() {
	let who = Sr25519Keyring::Bob.to_account_id();
	for call in governance_whitelist(&who) {
		assert!(ProxyType::Governance.filter(&call), "Governance must admit {call:?}");
	}
	for call in [transfer_call(&who), contract_call(), remark_call()] {
		assert!(!ProxyType::Governance.filter(&call), "Governance must block {call:?}");
	}
}

#[test]
fn proxy_type_lattice_orders_permissions() {
	use ProxyType::{Any, Governance, NonTransfer};

	assert!(Any.is_superset(&Any));
	assert!(Any.is_superset(&NonTransfer));
	assert!(Any.is_superset(&Governance));
	assert!(NonTransfer.is_superset(&Governance));
	assert!(!NonTransfer.is_superset(&Any));
	assert!(!Governance.is_superset(&NonTransfer));
	assert!(!Governance.is_superset(&Any));
}

#[test]
fn non_transfer_proxy_cannot_move_funds_on_chain() {
	new_test_ext().execute_with(|| {
		let delegator = Sr25519Keyring::Alice.to_account_id();
		let delegate = Sr25519Keyring::Bob.to_account_id();
		let target = Sr25519Keyring::Charlie.to_account_id();
		Balances::set_balance(&delegator, 100 * UNIT);
		assert_ok!(Proxy::add_proxy(
			RuntimeOrigin::signed(delegator.clone()),
			src(&delegate),
			ProxyType::NonTransfer,
			0,
		));
		let delegator_free = Balances::free_balance(&delegator);

		assert_ok!(Proxy::proxy(
			RuntimeOrigin::signed(delegate),
			src(&delegator),
			None,
			Box::new(transfer_call(&target)),
		));

		System::assert_last_event(
			pallet_proxy::Event::ProxyExecuted {
				result: Err(frame_system::Error::<Runtime>::CallFiltered.into()),
			}
			.into(),
		);
		assert_eq!(Balances::free_balance(&target), 0, "no funds may leave through the proxy");
		assert_eq!(Balances::free_balance(&delegator), delegator_free);
	});
}

#[test]
fn non_transfer_proxy_still_dispatches_harmless_calls() {
	new_test_ext().execute_with(|| {
		let delegator = Sr25519Keyring::Alice.to_account_id();
		let delegate = Sr25519Keyring::Bob.to_account_id();
		Balances::set_balance(&delegator, 100 * UNIT);
		assert_ok!(Proxy::add_proxy(
			RuntimeOrigin::signed(delegator.clone()),
			src(&delegate),
			ProxyType::NonTransfer,
			0,
		));

		assert_ok!(Proxy::proxy(
			RuntimeOrigin::signed(delegate),
			src(&delegator),
			None,
			Box::new(remark_call()),
		));

		System::assert_last_event(
			pallet_proxy::Event::ProxyExecuted { result: Ok(()) }.into(),
		);
	});
}
