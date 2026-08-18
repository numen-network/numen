//! On chain identity fields.
//!
//! Only x, telegram and discord gate qualified identity.
//! The others are only for contact and carry no weight in any gate.

use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
#[cfg(feature = "runtime-benchmarks")]
use enumflags2::BitFlag;
use enumflags2::{bitflags, BitFlags};
use pallet_identity::{Data, IdentityInformationProvider};
use scale_info::{build::Variants, Path, Type, TypeInfo};

/// One flag per field of [`IdentityInfo`]. Registrars use these to state which
/// fields a judgement covers.
#[bitflags]
#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdentityField {
	Display,
	Web,
	Email,
	Matrix,
	Github,
	X,
	Telegram,
	Discord,
}

impl TypeInfo for IdentityField {
	type Identity = Self;

	fn type_info() -> Type {
		Type::builder().path(Path::new("IdentityField", module_path!())).variant(
			Variants::new()
				.variant("Display", |v| v.index(0))
				.variant("Web", |v| v.index(1))
				.variant("Email", |v| v.index(2))
				.variant("Matrix", |v| v.index(3))
				.variant("Github", |v| v.index(4))
				.variant("X", |v| v.index(5))
				.variant("Telegram", |v| v.index(6))
				.variant("Discord", |v| v.index(7)),
		)
	}
}

/// Identity information the holder of an account registers about themselves.
#[derive(
	Clone,
	Debug,
	Decode,
	DecodeWithMemTracking,
	Default,
	Encode,
	Eq,
	MaxEncodedLen,
	PartialEq,
	TypeInfo,
)]
pub struct IdentityInfo {
	/// name the account goes by.
	pub display: Data,
	/// website. `https://` is prepended.
	pub web: Data,
	/// email address.
	pub email: Data,
	/// Matrix handle, in the `@user:server` form.
	pub matrix: Data,
	/// GitHub username.
	pub github: Data,
	/// X handle. The leading `@` may be elided.
	pub x: Data,
	/// Telegram handle. The leading `@` may be elided.
	pub telegram: Data,
	/// Discord handle.
	pub discord: Data,
}

impl IdentityInfo {
	fn fields(&self) -> BitFlags<IdentityField> {
		let mut fields = BitFlags::<IdentityField>::empty();
		if !self.display.is_none() {
			fields.insert(IdentityField::Display);
		}
		if !self.web.is_none() {
			fields.insert(IdentityField::Web);
		}
		if !self.email.is_none() {
			fields.insert(IdentityField::Email);
		}
		if !self.matrix.is_none() {
			fields.insert(IdentityField::Matrix);
		}
		if !self.github.is_none() {
			fields.insert(IdentityField::Github);
		}
		if !self.x.is_none() {
			fields.insert(IdentityField::X);
		}
		if !self.telegram.is_none() {
			fields.insert(IdentityField::Telegram);
		}
		if !self.discord.is_none() {
			fields.insert(IdentityField::Discord);
		}
		fields
	}
}

impl IdentityInformationProvider for IdentityInfo {
	type FieldsIdentifier = u64;

	fn has_identity(&self, fields: Self::FieldsIdentifier) -> bool {
		self.fields().bits() & fields == fields
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn create_identity_info() -> Self {
		let data = Data::Raw(alloc::vec![0; 32].try_into().expect("32 bytes fit a Data field"));

		IdentityInfo {
			display: data.clone(),
			web: data.clone(),
			email: data.clone(),
			matrix: data.clone(),
			github: data.clone(),
			x: data.clone(),
			telegram: data.clone(),
			discord: data,
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn all_fields() -> Self::FieldsIdentifier {
		IdentityField::all().bits()
	}
}
