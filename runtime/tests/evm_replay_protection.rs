// Replay protection for signed Ethereum transactions.
//
// A zero existential deposit keeps a drained account in state, so its nonce
// never rewinds to zero. Here the sender drains its balance through the
// balances-erc20 precompile yet keeps its nonce, so the pool and the block both
// reject the original signed bytes as stale once the address holds funds again.

mod common;

use common::{balances_erc20, encode_transfer, evm_account, new_test_ext};
use ethereum::{
	eip2930::TransactionSignature, EIP1559Transaction, EIP1559TransactionMessage, TransactionAction,
};
use frame_support::traits::{tokens::fungible::Mutate, Get};
use numen_runtime::{
	AccountId, Balances, Executive, Runtime, RuntimeCall, System, UncheckedExtrinsic, UNIT,
};
use pallet_ethereum::Transaction;
use pallet_evm::FeeCalculator;
use sp_core::{ecdsa, Pair, H160, H256, U256};
use sp_runtime::transaction_validity::{
	InvalidTransaction, TransactionSource, TransactionValidityError,
};

const GAS_LIMIT: u64 = 200_000;
/// Native balance the ERC20 drain moves out.
const DRAIN: u128 = 5_000 * UNIT;

fn base_fee() -> U256 {
	<Runtime as pallet_evm::Config>::FeeCalculator::min_gas_price().0
}

/// Gas prepay on top of the drain, so the drain leaves nothing behind.
fn funding() -> u128 {
	DRAIN + GAS_LIMIT as u128 * base_fee().as_u128()
}

/// Sign a tipless EIP-1559 ERC20 transfer of `amount` to `to` at the current
/// base fee.
fn signed_drain(pair: &ecdsa::Pair, nonce: u64, to: H160, amount: u128) -> Transaction {
	let message = EIP1559TransactionMessage {
		chain_id: <Runtime as pallet_evm::Config>::ChainId::get(),
		nonce: U256::from(nonce),
		max_priority_fee_per_gas: U256::zero(),
		max_fee_per_gas: base_fee(),
		gas_limit: U256::from(GAS_LIMIT),
		action: TransactionAction::Call(balances_erc20()),
		value: U256::zero(),
		input: encode_transfer(to, amount),
		access_list: Vec::new(),
	};

	let raw = pair.sign_prehashed(message.hash().as_fixed_bytes());
	let bytes: &[u8] = raw.as_ref();
	let signature = TransactionSignature::new(
		bytes[64] == 1,
		H256::from_slice(&bytes[0..32]),
		H256::from_slice(&bytes[32..64]),
	)
	.expect("signature components are in range");

	Transaction::EIP1559(EIP1559Transaction {
		chain_id: message.chain_id,
		nonce: message.nonce,
		max_priority_fee_per_gas: message.max_priority_fee_per_gas,
		max_fee_per_gas: message.max_fee_per_gas,
		gas_limit: message.gas_limit,
		action: message.action,
		value: message.value,
		input: message.input,
		access_list: message.access_list,
		signature,
	})
}

/// Recover the sender through the self-contained signature check.
fn recovered_sender(tx: &Transaction) -> H160 {
	pallet_ethereum::Call::<Runtime>::transact { transaction: tx.clone() }
		.check_self_contained()
		.expect("transact is self-contained")
		.expect("signature recovers a sender")
}

fn ethereum_extrinsic(tx: &Transaction) -> UncheckedExtrinsic {
	UncheckedExtrinsic::new_bare(RuntimeCall::Ethereum(pallet_ethereum::Call::transact {
		transaction: tx.clone(),
	}))
}

/// Fund the signer, apply its nonce-0 drain into `recipient` and return the
/// drained account with the signed bytes an attacker would replay.
fn drain(pair: &ecdsa::Pair, recipient: H160) -> (AccountId, Transaction) {
	let drain = signed_drain(pair, 0, recipient, DRAIN);
	let sender_acc = evm_account(recovered_sender(&drain));
	Balances::set_balance(&sender_acc, funding());
	Executive::apply_extrinsic(ethereum_extrinsic(&drain))
		.expect("the drain passes the in-block check")
		.expect("the drain dispatches");
	assert_eq!(Balances::free_balance(evm_account(recipient)), DRAIN, "the drain executed in full");
	(sender_acc, drain)
}

#[test]
fn drained_sender_keeps_its_nonce_so_the_signed_transaction_cannot_replay() {
	new_test_ext().execute_with(|| {
		let pair = ecdsa::Pair::from_seed(&[0x11u8; 32]);
		let recipient = H160::from_low_u64_be(0xBEEF);
		let (sender_acc, drain) = drain(&pair, recipient);

		assert!(System::account_exists(&sender_acc), "the drained account stays in state");
		assert_eq!(
			System::account_nonce(&sender_acc),
			1,
			"the drained sender keeps its advanced nonce"
		);

		Balances::set_balance(&sender_acc, funding());
		assert_eq!(
			Executive::apply_extrinsic(ethereum_extrinsic(&drain)),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
			"the block rejects the replayed bytes as stale"
		);
		assert_eq!(Balances::free_balance(&sender_acc), funding(), "the rejected replay costs nothing");
		assert_eq!(Balances::free_balance(evm_account(recipient)), DRAIN, "the recipient is paid once");
	});
}

#[test]
fn the_pool_rejects_the_replayed_transaction_as_stale() {
	new_test_ext().execute_with(|| {
		let pair = ecdsa::Pair::from_seed(&[0x11u8; 32]);
		let recipient = H160::from_low_u64_be(0xBEEF);
		let (sender_acc, drain) = drain(&pair, recipient);

		Balances::set_balance(&sender_acc, funding());
		assert_eq!(
			Executive::validate_transaction(
				TransactionSource::External,
				ethereum_extrinsic(&drain),
				Default::default(),
			),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
			"the pool rejects the replayed bytes as stale"
		);
	});
}

#[test]
fn drained_sender_spends_again_with_its_next_nonce() {
	new_test_ext().execute_with(|| {
		let pair = ecdsa::Pair::from_seed(&[0x11u8; 32]);
		let recipient = H160::from_low_u64_be(0xBEEF);
		let (sender_acc, _) = drain(&pair, recipient);

		Balances::set_balance(&sender_acc, funding());
		Executive::apply_extrinsic(ethereum_extrinsic(&signed_drain(&pair, 1, recipient, DRAIN)))
			.expect("the next nonce passes the in-block check")
			.expect("the next nonce dispatches");
		assert_eq!(
			Balances::free_balance(evm_account(recipient)),
			2 * DRAIN,
			"the refilled account spends again"
		);
		assert_eq!(System::account_nonce(&sender_acc), 2);
	});
}
