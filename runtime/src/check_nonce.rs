//! Nonce check that keeps every account that ever signed alive. It starts an
//! Ethereum signer from the nonce its H160 reached under Frontier.

use alloc::vec::Vec;
use codec::{Decode, DecodeWithMemTracking, Encode};
use frame_support::{dispatch::DispatchInfo, traits::Get, weights::Weight};
use sp_runtime::{
	traits::{
		AsSystemOriginSigner, DispatchInfoOf, Dispatchable, Implication, PostDispatchInfoOf,
		TransactionExtension, TransactionExtensionMetadata, ValidateResult, Zero,
	},
	transaction_validity::{TransactionSource, TransactionValidityError},
	DispatchResult,
};

type Inner<T> = frame_system::CheckNonce<T>;
type InnerVal<T> =
	<Inner<T> as TransactionExtension<<T as frame_system::Config>::RuntimeCall>>::Val;
type InnerPre<T> =
	<Inner<T> as TransactionExtension<<T as frame_system::Config>::RuntimeCall>>::Pre;

/// [`frame_system::CheckNonce`] that also gives an account a provider of its own
/// the first time it signs.
///
/// Reaping an account resets its nonce to zero, after which every transaction it
/// signed before is valid again. The provider is never released, so an account
/// that signed once outlives a drain and its nonce keeps counting. Encoding,
/// identifier and type information are those of the wrapped check, so signers
/// and metadata readers see no difference.
///
/// An Ethereum signer's first transaction under pallet-revive is checked against
/// the nonce its H160 reached under Frontier. The transactions Frontier ran for
/// it thus stay spent, claimed or not.
#[derive(Encode, Decode, DecodeWithMemTracking, Clone, Eq, PartialEq)]
pub struct CheckNonceKeepSigned<T: frame_system::Config>(pub Inner<T>);

impl<T: frame_system::Config> CheckNonceKeepSigned<T> {
	pub fn from(nonce: T::Nonce) -> Self {
		Self(Inner::from(nonce))
	}
}

impl<T: frame_system::Config> scale_info::TypeInfo for CheckNonceKeepSigned<T> {
	type Identity = Inner<T>;

	fn type_info() -> scale_info::Type {
		Inner::<T>::type_info()
	}
}

impl<T: frame_system::Config> core::fmt::Debug for CheckNonceKeepSigned<T> {
	fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
		self.0.fmt(f)
	}
}

impl<T: pallet_evm_claims::Config> TransactionExtension<T::RuntimeCall> for CheckNonceKeepSigned<T>
where
	T::RuntimeCall: Dispatchable<Info = DispatchInfo>,
	<T::RuntimeCall as Dispatchable>::RuntimeOrigin: AsSystemOriginSigner<T::AccountId> + Clone,
{
	const IDENTIFIER: &'static str = <Inner<T> as TransactionExtension<T::RuntimeCall>>::IDENTIFIER;
	type Implicit = ();
	type Val = (Option<T::AccountId>, InnerVal<T>);
	type Pre = InnerPre<T>;

	fn metadata() -> Vec<TransactionExtensionMetadata> {
		<Inner<T> as TransactionExtension<T::RuntimeCall>>::metadata()
	}

	fn weight(&self, call: &T::RuntimeCall) -> Weight {
		self.0.weight(call).saturating_add(T::DbWeight::get().reads(1))
	}

	fn validate(
		&self,
		origin: <T as frame_system::Config>::RuntimeOrigin,
		call: &T::RuntimeCall,
		info: &DispatchInfoOf<T::RuntimeCall>,
		len: usize,
		self_implicit: Self::Implicit,
		inherited_implication: &impl Implication,
		source: TransactionSource,
	) -> ValidateResult<Self::Val, T::RuntimeCall> {
		let signer = origin.as_system_origin_signer().cloned();
		// Writes here vanish with the pool's validation overlay. In a block they
		// are the lift itself, made before the wrapped check reads the nonce.
		if let Some(who) = &signer {
			pallet_evm_claims::Pallet::<T>::lift_nonce(who);
		}
		let (validity, val, origin) =
			self.0.validate(origin, call, info, len, self_implicit, inherited_implication, source)?;
		Ok((validity, (signer, val), origin))
	}

	fn prepare(
		self,
		(signer, val): Self::Val,
		origin: &<T as frame_system::Config>::RuntimeOrigin,
		call: &T::RuntimeCall,
		info: &DispatchInfoOf<T::RuntimeCall>,
		len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		let first = signer.filter(|who| frame_system::Pallet::<T>::account_nonce(who).is_zero());
		let pre = self.0.prepare(val, origin, call, info, len)?;
		if let Some(who) = first {
			frame_system::Pallet::<T>::inc_providers(&who);
		}
		Ok(pre)
	}

	fn post_dispatch_details(
		pre: Self::Pre,
		info: &DispatchInfoOf<T::RuntimeCall>,
		post_info: &PostDispatchInfoOf<T::RuntimeCall>,
		len: usize,
		result: &DispatchResult,
	) -> Result<Weight, TransactionValidityError> {
		<Inner<T> as TransactionExtension<T::RuntimeCall>>::post_dispatch_details(
			pre, info, post_info, len, result,
		)
	}
}
