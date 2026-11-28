//! Ethereum EOA calls into whitelisted runtime extrinsics via `DispatchPrecompile`.

mod common;

use codec::Encode;
use common::preset_ext;
use frame_support::{
	assert_ok,
	storage::with_transaction,
	traits::{schedule::DispatchTime, Bounded, Get, OriginTrait},
	weights::Weight,
};
use numen_runtime::{
	configs::{
		evm::{EvmChainId, GasScale},
		governance::{pallet_custom_origins, TracksInfo},
	},
	dispatch_precompile::{self, IDispatch, DECODE_DEPTH_LIMIT},
	AccountId, Balance, Balances, Executive, Referenda, Revive, Runtime, RuntimeCall, RuntimeEvent,
	RuntimeOrigin, System, Treasury, UncheckedExtrinsic, Vesting, UNIT,
};
use pallet_conviction_voting::{AccountVote, Conviction, Vote, Voting};
use pallet_referenda::{ReferendumInfo, ReferendumInfoFor, TracksInfo as _};
use pallet_revive::{
	evm::{Account, GenericTransaction},
	precompiles::alloy::sol_types::{Revert, SolCall, SolError},
	Code, ExecConfig, TransactionLimits, U256,
};
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{generic, DispatchError, MultiAddress, TransactionOutcome};

fn dev_ext() -> TestExternalities {
	let mut ext = preset_ext(sp_genesis_builder::DEV_RUNTIME_PRESET);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

fn dispatch_tx(from: &Account, call: &RuntimeCall, value: U256) -> GenericTransaction {
	let data = IDispatch::dispatchCall { call: call.encode().into() }.abi_encode();
	GenericTransaction {
		from: Some(from.address()),
		to: Some(dispatch_precompile::ADDRESS),
		input: data.into(),
		value: Some(value),
		chain_id: Some(U256::from(EvmChainId::get())),
		nonce: Some(System::account_nonce(from.substrate_account()).into()),
		gas_price: Some(Revive::evm_base_fee()),
		..Default::default()
	}
}

fn raw_tx(from: &Account, input: Vec<u8>, value: U256) -> GenericTransaction {
	GenericTransaction {
		from: Some(from.address()),
		to: Some(dispatch_precompile::ADDRESS),
		input: input.into(),
		value: Some(value),
		chain_id: Some(U256::from(EvmChainId::get())),
		nonce: Some(System::account_nonce(from.substrate_account()).into()),
		gas_price: Some(Revive::evm_base_fee()),
		..Default::default()
	}
}

fn dry_run_result(
	tx: GenericTransaction,
) -> Result<pallet_revive::EthTransactInfo<Balance>, pallet_revive::EthTransactError> {
	with_transaction(|| {
		TransactionOutcome::Rollback(Ok::<_, DispatchError>(Revive::dry_run_eth_transact(
			tx, None, true, None,
		)))
	})
	.expect("rollback")
}

fn eth_xt(from: &Account, mut tx: GenericTransaction) -> UncheckedExtrinsic {
	let gas = dry_run_result(tx.clone()).expect("dry run succeeds").eth_gas;
	tx.gas = Some(gas);
	let signed = from.sign_transaction(tx.try_into_unsigned().expect("unsigned tx"));
	let call =
		RuntimeCall::Revive(pallet_revive::Call::eth_transact { payload: signed.signed_payload() });
	generic::UncheckedExtrinsic::new_bare(call).into()
}

fn eth_xt_with_gas(from: &Account, mut tx: GenericTransaction, gas: u64) -> UncheckedExtrinsic {
	tx.gas = Some(U256::from(gas));
	let signed = from.sign_transaction(tx.try_into_unsigned().expect("unsigned tx"));
	let call =
		RuntimeCall::Revive(pallet_revive::Call::eth_transact { payload: signed.signed_payload() });
	generic::UncheckedExtrinsic::new_bare(call).into()
}

fn revert_reason(err: pallet_revive::EthTransactError) -> String {
	match err {
		pallet_revive::EthTransactError::Data(data) => {
			Revert::abi_decode(&data).map(|r| r.reason).unwrap_or_default()
		},
		pallet_revive::EthTransactError::Message(msg) => msg,
	}
}

fn qualify(who: &AccountId) {
	pallet_identity::IdentityOf::<Runtime>::insert(
		who,
		pallet_identity::Registration {
			judgements: vec![(0, pallet_identity::Judgement::Reasonable)]
				.try_into()
				.expect("fits bound"),
			deposit: 0,
			info: numen_runtime::identity_info::IdentityInfo {
				x: b"proposer".to_vec().try_into().expect("fits bound"),
				..Default::default()
			},
		},
	);
}

fn open_referendum(proposer: &AccountId) -> (u32, u16) {
	qualify(proposer);
	let origin = RuntimeOrigin::from(pallet_custom_origins::Origin::WishForChange);
	let track = TracksInfo::track_for(origin.caller()).expect("track exists");
	let index = pallet_referenda::ReferendumCount::<Runtime>::get();
	let remark = RuntimeCall::System(frame_system::Call::remark { remark: b"poll".to_vec() });
	assert_ok!(Referenda::submit(
		RuntimeOrigin::signed(proposer.clone()),
		Box::new(origin.caller().clone()),
		Bounded::Inline(remark.encode().try_into().expect("inline")),
		DispatchTime::After(10),
	));
	(index, track)
}

fn limits() -> TransactionLimits<Runtime> {
	TransactionLimits::WeightAndDeposit {
		weight_limit: Weight::from_parts(500_000_000_000, 5 * 1024 * 1024),
		deposit_limit: 10 * UNIT,
	}
}

#[test]
fn an_eth_signer_places_deposit_batches_votes_and_unlocks() {
	dev_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let (first, track) = open_referendum(&alice);
		let (second, _) = open_referendum(&alice);
		let alith = Account::default();
		let who = alith.substrate_account();
		let start_evm = Revive::evm_balance(&alith.address());

		let place = RuntimeCall::Referenda(pallet_referenda::Call::place_decision_deposit {
			index: first,
		});
		Executive::apply_extrinsic(eth_xt(&alith, dispatch_tx(&alith, &place, U256::zero())))
			.expect("pool accepts")
			.expect("extrinsic runs");

		let deposit = TracksInfo::info(track).expect("track").decision_deposit;
		assert_eq!(Balances::reserved_balance(&who), deposit);

		let extra = 500 * UNIT;
		let stake = deposit + extra;
		let abstain = 200 * UNIT;
		let vote_batch = RuntimeCall::Utility(pallet_utility::Call::batch_all {
			calls: vec![
				RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
					poll_index: first,
					vote: AccountVote::Standard {
						vote: Vote { aye: true, conviction: Conviction::Locked1x },
						balance: stake,
					},
				}),
				RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
					poll_index: second,
					vote: AccountVote::SplitAbstain { aye: 0, nay: 0, abstain },
				}),
			],
		});
		Executive::apply_extrinsic(eth_xt(&alith, dispatch_tx(&alith, &vote_batch, U256::zero())))
			.expect("pool accepts")
			.expect("extrinsic runs");

		let ReferendumInfo::Ongoing(status_one) = ReferendumInfoFor::<Runtime>::get(first).unwrap()
		else {
			panic!("first ongoing");
		};
		let ReferendumInfo::Ongoing(status_two) =
			ReferendumInfoFor::<Runtime>::get(second).unwrap()
		else {
			panic!("second ongoing");
		};
		assert_eq!(status_one.tally.ayes, stake);
		assert_eq!(status_one.tally.support, stake);
		assert_eq!(status_two.tally.support, abstain);
		// The reserve already covers `deposit` of the freeze, so spendable drops by
		// max(stake, deposit) rather than stake + deposit.
		assert!(Revive::evm_balance(&alith.address()) <= start_evm - U256::from(stake.max(deposit)));

		let clear = RuntimeCall::Utility(pallet_utility::Call::batch_all {
			calls: vec![
				RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::remove_vote {
					class: Some(track),
					index: first,
				}),
				RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::remove_vote {
					class: Some(track),
					index: second,
				}),
				RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::unlock {
					class: track,
					target: MultiAddress::Id(who.clone()),
				}),
			],
		});
		let before_clear = Revive::evm_balance(&alith.address());
		Executive::apply_extrinsic(eth_xt(&alith, dispatch_tx(&alith, &clear, U256::zero())))
			.expect("pool accepts")
			.expect("extrinsic runs");

		assert!(pallet_conviction_voting::ClassLocksFor::<Runtime>::get(&who).is_empty());
		assert!(Revive::evm_balance(&alith.address()) > before_clear + U256::from(extra - UNIT));
		assert!(System::events().iter().any(|r| matches!(
			r.event,
			RuntimeEvent::ConvictionVoting(pallet_conviction_voting::Event::VoteUnlocked { .. })
		)));
	});
}

#[test]
fn an_eth_signer_delegates_to_an_unmapped_substrate_account_and_undelegates() {
	dev_ext().execute_with(|| {
		let alith = Account::default();
		let who = alith.substrate_account();
		let delegatee = AccountId::from([0x77; 32]);
		let track = 2u16;
		let amount = 300 * UNIT;

		let delegate = RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::delegate {
			class: track,
			to: MultiAddress::Id(delegatee.clone()),
			conviction: Conviction::Locked2x,
			balance: amount,
		});
		Executive::apply_extrinsic(eth_xt(&alith, dispatch_tx(&alith, &delegate, U256::zero())))
			.expect("pool accepts")
			.expect("extrinsic runs");

		match pallet_conviction_voting::VotingFor::<Runtime>::get(&who, track) {
			Voting::Delegating(d) => {
				assert_eq!(d.target, delegatee);
				assert_eq!(d.balance, amount);
				assert_eq!(d.conviction, Conviction::Locked2x);
			},
			Voting::Casting(_) => panic!("expected delegating state"),
		}

		let undelegate =
			RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::undelegate {
				class: track,
			});
		Executive::apply_extrinsic(eth_xt(&alith, dispatch_tx(&alith, &undelegate, U256::zero())))
			.expect("pool accepts")
			.expect("extrinsic runs");

		assert!(matches!(
			pallet_conviction_voting::VotingFor::<Runtime>::get(&who, track),
			Voting::Casting(_)
		));
	});
}

#[test]
fn an_eth_signer_claims_payout_refunds_deposit_and_vests() {
	dev_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let (poll, _) = open_referendum(&alice);
		let canceller = RuntimeOrigin::from(pallet_custom_origins::Origin::ReferendumCanceller);
		assert_ok!(Referenda::cancel(canceller, poll));
		let reserved_before = Balances::reserved_balance(&alice);
		assert!(reserved_before > 0);

		let spender = RuntimeOrigin::from(pallet_custom_origins::Origin::SmallSpender);
		let beneficiary = Sr25519Keyring::Bob.to_account_id();
		let bob_before = Balances::free_balance(&beneficiary);
		let payout_amount = 50 * UNIT;
		assert_ok!(Treasury::spend(
			spender,
			Box::new(()),
			payout_amount,
			Box::new(beneficiary.clone()),
			None,
		));

		let alith = Account::default();
		let who = alith.substrate_account();
		assert_ok!(Vesting::vested_transfer(
			RuntimeOrigin::signed(alice.clone()),
			MultiAddress::Id(who.clone()),
			pallet_vesting::VestingInfo::new(100 * UNIT, 10 * UNIT, 1),
		));
		System::set_block_number(12);

		let sweep = RuntimeCall::Utility(pallet_utility::Call::batch_all {
			calls: vec![
				RuntimeCall::Referenda(pallet_referenda::Call::refund_submission_deposit {
					index: poll,
				}),
				RuntimeCall::Treasury(pallet_treasury::Call::payout { index: 0 }),
				RuntimeCall::Vesting(pallet_vesting::Call::vest {}),
			],
		});
		Executive::apply_extrinsic(eth_xt(&alith, dispatch_tx(&alith, &sweep, U256::zero())))
			.expect("pool accepts")
			.expect("extrinsic runs");

		assert_eq!(Balances::reserved_balance(&alice), 0);
		assert_eq!(Balances::free_balance(&beneficiary), bob_before + payout_amount);
		assert!(Vesting::vesting(who).is_none());
	});
}

#[test]
fn disallowed_calls_revert_and_leave_state_untouched() {
	dev_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let (poll, _) = open_referendum(&alice);
		let alith = Account::default();
		let bob = Sr25519Keyring::Bob.to_account_id();
		let bob_before = Balances::free_balance(&bob);

		let transfer = RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive {
			dest: MultiAddress::Id(bob.clone()),
			value: UNIT,
		});
		let mixed = RuntimeCall::Utility(pallet_utility::Call::batch_all {
			calls: vec![
				RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
					poll_index: poll,
					vote: AccountVote::Standard {
						vote: Vote { aye: true, conviction: Conviction::Locked1x },
						balance: 10 * UNIT,
					},
				}),
				transfer.clone(),
			],
		});
		let propose = RuntimeCall::Referenda(pallet_referenda::Call::submit {
			proposal_origin: Box::new(
				RuntimeOrigin::from(pallet_custom_origins::Origin::WishForChange).caller().clone(),
			),
			proposal: Bounded::Inline(vec![0u8; 4].try_into().expect("inline")),
			enactment_moment: DispatchTime::After(10),
		});

		for forbidden in [transfer, mixed, propose] {
			let err =
				dry_run_result(dispatch_tx(&alith, &forbidden, U256::zero())).expect_err("reverts");
			assert_eq!(revert_reason(err), "Call not allowed");
		}

		let forced = dispatch_tx(
			&alith,
			&RuntimeCall::Utility(pallet_utility::Call::batch_all {
				calls: vec![
					RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
						poll_index: poll,
						vote: AccountVote::Standard {
							vote: Vote { aye: true, conviction: Conviction::Locked1x },
							balance: 10 * UNIT,
						},
					}),
					RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive {
						dest: MultiAddress::Id(bob.clone()),
						value: UNIT,
					}),
				],
			}),
			U256::zero(),
		);
		Executive::apply_extrinsic(eth_xt_with_gas(&alith, forced, 500_000))
			.expect("pool accepts")
			.expect("eth_transact records a reverted receipt");

		let ReferendumInfo::Ongoing(status) = ReferendumInfoFor::<Runtime>::get(poll).unwrap()
		else {
			panic!("ongoing");
		};
		assert_eq!(status.tally.ayes, 0);
		assert_eq!(Balances::free_balance(&bob), bob_before);
	});
}

#[test]
fn value_transfers_staticcalls_delegatecalls_and_contract_callers_fail() {
	dev_ext().execute_with(|| {
		let alith = Account::default();
		let vest = RuntimeCall::Vesting(pallet_vesting::Call::vest {});
		let payload = IDispatch::dispatchCall { call: vest.encode().into() }.abi_encode();

		let payable_err =
			dry_run_result(dispatch_tx(&alith, &vest, U256::from(1u8))).expect_err("reverts");
		assert_eq!(revert_reason(payable_err), "Function is not payable");

		// Minimal EVM forwarders for CALL (0xf1), STATICCALL (0xfa), and DELEGATECALL (0xf4)
		// that return the 32-byte subcall status word (0 on revert, 1 on success).
		for opcode in [0xf1u8, 0xfau8, 0xf4u8] {
			let mut runtime_code = vec![
				0x36, 0x60, 0x00, 0x60, 0x00, 0x37,
				0x60, 0x00, 0x60, 0x00, 0x36, 0x60, 0x00,
			];
			if opcode == 0xf1 {
				runtime_code.extend([0x60, 0x00]);
			}
			runtime_code.extend([
				0x63, 0x08, 0x03, 0x00, 0x00,
				0x5a, opcode,
				0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
			]);

			let mut init = vec![
				0x60,
				runtime_code.len() as u8,
				0x60,
				0x0c,
				0x60,
				0x00,
				0x39,
				0x60,
				runtime_code.len() as u8,
				0x60,
				0x00,
				0xf3,
			];
			init.extend_from_slice(&runtime_code);

			let forwarder = Revive::bare_instantiate(
				RuntimeOrigin::signed(Sr25519Keyring::Alice.to_account_id()),
				U256::zero(),
				limits(),
				Code::Upload(init),
				Vec::new(),
				Some([opcode; 32]),
				&ExecConfig::new_substrate_tx(),
			)
			.result
			.expect("forwarder deploys")
			.addr;

			let out = Revive::bare_call(
				RuntimeOrigin::signed(Sr25519Keyring::Alice.to_account_id()),
				forwarder,
				U256::zero(),
				limits(),
				payload.clone(),
				&ExecConfig::new_substrate_tx(),
			)
			.result
			.expect("forwarder runs");
			assert!(!out.did_revert());
			assert_eq!(out.data, vec![0u8; 32], "opcode {opcode:#x} subcall must revert");
		}
	});
}

fn nested_batch(depth: u32, leaf: RuntimeCall) -> RuntimeCall {
	let mut current = leaf;
	for _ in 0..depth {
		current = RuntimeCall::Utility(pallet_utility::Call::batch { calls: vec![current] });
	}
	current
}

#[test]
fn nesting_at_the_decode_limit_runs_and_one_deeper_or_trailing_bytes_reverts() {
	dev_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let (poll, _) = open_referendum(&alice);
		let alith = Account::default();

		let leaf = RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
			poll_index: poll,
			vote: AccountVote::SplitAbstain { aye: 0, nay: 0, abstain: 10 * UNIT },
		});

		let mut max_batches = 0u32;
		while <RuntimeCall as codec::DecodeLimit>::decode_all_with_depth_limit(
			DECODE_DEPTH_LIMIT,
			&mut &nested_batch(max_batches + 1, leaf.clone()).encode()[..],
		)
		.is_ok()
		{
			max_batches += 1;
		}
		assert!(max_batches > 0);

		let at_limit = nested_batch(max_batches, leaf.clone());
		assert!(dry_run_result(dispatch_tx(&alith, &at_limit, U256::zero())).is_ok());

		let past_limit = nested_batch(max_batches + 1, leaf.clone());
		let deep_err =
			dry_run_result(dispatch_tx(&alith, &past_limit, U256::zero())).expect_err("too deep");
		assert_eq!(revert_reason(deep_err), "Invalid runtime call");

		let mut trailing = leaf.encode();
		trailing.push(0xff);
		let trailing_data = IDispatch::dispatchCall { call: trailing.into() }.abi_encode();
		let trailing_err =
			dry_run_result(raw_tx(&alith, trailing_data, U256::zero())).expect_err("trailing byte");
		assert_eq!(revert_reason(trailing_err), "Invalid runtime call");
	});
}

#[test]
fn unused_dispatch_weight_is_refunded_from_gas() {
	dev_ext().execute_with(|| {
		let alice = Sr25519Keyring::Alice.to_account_id();
		let (poll, _) = open_referendum(&alice);
		let alith = Account::default();

		let valid_vote = RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
			poll_index: poll,
			vote: AccountVote::SplitAbstain { aye: 0, nay: 0, abstain: UNIT },
		});
		let missing_vote = RuntimeCall::ConvictionVoting(pallet_conviction_voting::Call::vote {
			poll_index: 999,
			vote: AccountVote::SplitAbstain { aye: 0, nay: 0, abstain: UNIT },
		});

		// Both batches reserve 10 votes of pre-dispatch weight, yet the interrupted batch stops on
		// item two and returns actual weight for two calls.
		let mut interrupted_calls = vec![valid_vote.clone()];
		interrupted_calls.extend(core::iter::repeat_n(missing_vote, 9));
		let interrupted =
			RuntimeCall::Utility(pallet_utility::Call::batch { calls: interrupted_calls });
		let full = RuntimeCall::Utility(pallet_utility::Call::batch {
			calls: vec![valid_vote; 10],
		});

		let gas_full = dry_run_result(dispatch_tx(&alith, &full, U256::zero()))
			.expect("dry run")
			.eth_gas;
		let gas_interrupted = dry_run_result(dispatch_tx(&alith, &interrupted, U256::zero()))
			.expect("dry run")
			.eth_gas;
		// Dry-run reports the pre-dispatch peak so the signed gas limit clears `env.charge`,
		// whereas execution refunds the eight skipped calls via `env.adjust_gas`.
		assert!(gas_interrupted > U256::from(21_000));
		assert_eq!(gas_interrupted, gas_full);

		let gas_cap = (gas_full.as_u64()).saturating_mul(2);
		let before_interrupted = Balances::free_balance(alith.substrate_account());
		Executive::apply_extrinsic(eth_xt_with_gas(
			&alith,
			dispatch_tx(&alith, &interrupted, U256::zero()),
			gas_cap,
		))
		.expect("pool accepts")
		.expect("runs");
		let paid_interrupted = before_interrupted - Balances::free_balance(alith.substrate_account());

		let before_full = Balances::free_balance(alith.substrate_account());
		Executive::apply_extrinsic(eth_xt_with_gas(
			&alith,
			dispatch_tx(&alith, &full, U256::zero()),
			gas_cap,
		))
		.expect("pool accepts")
		.expect("runs");
		let paid_full = before_full - Balances::free_balance(alith.substrate_account());

		assert!(paid_interrupted * 2 < paid_full, "paid {paid_interrupted} vs {paid_full}");
		assert!(gas_full < U256::from(5_000_000u64 * u64::from(GasScale::get())));
	});
}
