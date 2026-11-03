//! Accounts that ever signed outlive a drain. The nonce check gives a signer a
//! provider of its own on its first transaction, an account that only received
//! funds gets none, and the wrapper stays invisible to signers and metadata.

mod common;

use codec::Encode;
use common::new_test_ext;
use frame_support::traits::tokens::{fungible::Mutate, Preservation};
use numen_runtime::{
	check_nonce::CheckNonceKeepSigned, AccountId, Balances, Executive, Runtime, RuntimeCall,
	SignedPayload, System, TxExtension, UncheckedExtrinsic, UNIT,
};
use sp_core::Pair;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	generic::Era,
	traits::{TransactionExtension, TransactionExtensionMetadata},
	MultiAddress, MultiSignature,
};

const FUNDING: u128 = 1_000 * UNIT;

fn extension(nonce: u32) -> TxExtension {
	(
		frame_system::AuthorizeCall::<Runtime>::new(),
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(Era::Immortal),
		CheckNonceKeepSigned::<Runtime>::from(nonce),
		frame_system::CheckWeight::<Runtime>::new(),
		pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
		frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
		frame_system::WeightReclaim::<Runtime>::new(),
	)
}

fn remark(signer: Sr25519Keyring, nonce: u32) -> UncheckedExtrinsic {
	let call = RuntimeCall::System(frame_system::Call::remark { remark: Vec::new() });
	let payload = SignedPayload::new(call, extension(nonce)).expect("implicit data resolves");
	let signature = payload.using_encoded(|bytes| signer.pair().sign(bytes));
	let (call, extension, _) = payload.deconstruct();
	UncheckedExtrinsic::new_signed(
		call,
		MultiAddress::Id(signer.to_account_id()),
		MultiSignature::Sr25519(signature),
		extension,
	)
}

#[test]
fn the_first_signed_transaction_pins_the_signer_once() {
	new_test_ext().execute_with(|| {
		let signer = Sr25519Keyring::Ferdie;
		let who = signer.to_account_id();
		Balances::set_balance(&who, FUNDING);
		let providers = System::providers(&who);

		Executive::apply_extrinsic(remark(signer, 0))
			.expect("the first remark is valid")
			.expect("the first remark dispatches");
		assert_eq!(System::providers(&who), providers + 1, "the first signature adds a provider");

		Executive::apply_extrinsic(remark(signer, 1))
			.expect("the second remark is valid")
			.expect("the second remark dispatches");
		assert_eq!(System::providers(&who), providers + 1, "later signatures add none");
	});
}

#[test]
fn an_account_that_only_received_funds_is_reaped_when_drained() {
	new_test_ext().execute_with(|| {
		let receiver = Sr25519Keyring::Dave.to_account_id();
		let sink: AccountId = Sr25519Keyring::Eve.to_account_id();
		Balances::set_balance(&receiver, FUNDING);
		assert_eq!(System::providers(&receiver), 1, "receiving funds pins nothing");

		<Balances as Mutate<AccountId>>::transfer(&receiver, &sink, FUNDING, Preservation::Expendable)
			.expect("the drain goes through");

		assert!(!System::account_exists(&receiver), "a drained account that never signed goes");
	});
}

/// The signed extension list as stock `CheckNonce` would publish it.
type PlainExtension = (
	frame_system::AuthorizeCall<Runtime>,
	frame_system::CheckNonZeroSender<Runtime>,
	frame_system::CheckSpecVersion<Runtime>,
	frame_system::CheckTxVersion<Runtime>,
	frame_system::CheckGenesis<Runtime>,
	frame_system::CheckEra<Runtime>,
	frame_system::CheckNonce<Runtime>,
	frame_system::CheckWeight<Runtime>,
	pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
	frame_metadata_hash_extension::CheckMetadataHash<Runtime>,
	frame_system::WeightReclaim<Runtime>,
);

/// Extension identifiers and type layouts in the form a metadata reader sees.
fn published<X>() -> Vec<u8>
where
	X: scale_info::StaticTypeInfo + TransactionExtension<RuntimeCall>,
{
	let mut registry = scale_info::Registry::new();
	let tuple = registry.register_type(&scale_info::meta_type::<X>()).id;
	let entries: Vec<(String, u32, u32)> = X::metadata()
		.into_iter()
		.map(|TransactionExtensionMetadata { identifier, ty, implicit }| {
			(identifier.to_string(), registry.register_type(&ty).id, registry.register_type(&implicit).id)
		})
		.collect();
	(tuple, entries, scale_info::PortableRegistry::from(registry)).encode()
}

#[test]
fn signers_and_metadata_readers_see_the_stock_nonce_check() {
	assert_eq!(published::<TxExtension>(), published::<PlainExtension>());
	assert_eq!(
		CheckNonceKeepSigned::<Runtime>::from(300).encode(),
		frame_system::CheckNonce::<Runtime>::from(300).encode(),
	);
}
