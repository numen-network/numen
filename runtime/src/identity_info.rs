//! On chain identity fields.
//!
//! Only x, telegram and discord gate qualified identity. Everything else is
//! self description and contact detail carrying no weight in any gate.

use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
#[cfg(feature = "runtime-benchmarks")]
use enumflags2::BitFlag;
use enumflags2::{bitflags, BitFlags};
use frame_support::{traits::ConstU32, BoundedVec};
use pallet_identity::IdentityInformationProvider;
use scale_info::{build::Variants, Path, Type, TypeInfo};

/// Bytes an account writes about itself, bounded at `N`. Empty means the field
/// was never filled in.
pub type Text<const N: u32> = BoundedVec<u8, ConstU32<N>>;

/// One flag per field of [`IdentityInfo`]. Registrars use these to state which
/// fields a judgement covers.
#[bitflags]
#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdentityField {
	Display,
	Avatar,
	About,
	Web,
	Email,
	Github,
	Matrix,
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
				.variant("Avatar", |v| v.index(1))
				.variant("About", |v| v.index(2))
				.variant("Web", |v| v.index(3))
				.variant("Email", |v| v.index(4))
				.variant("Github", |v| v.index(5))
				.variant("Matrix", |v| v.index(6))
				.variant("X", |v| v.index(7))
				.variant("Telegram", |v| v.index(8))
				.variant("Discord", |v| v.index(9)),
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
	pub display: Text<32>,
	/// picture URL.
	pub avatar: Text<128>,
	/// self description.
	pub about: Text<2048>,
	/// website. `https://` is prepended.
	pub web: Text<64>,
	/// email address.
	pub email: Text<32>,
	/// GitHub username.
	pub github: Text<32>,
	/// Matrix handle, in the `@user:server` form.
	pub matrix: Text<32>,
	/// X handle. The leading `@` may be elided.
	pub x: Text<32>,
	/// Telegram handle. The leading `@` may be elided.
	pub telegram: Text<32>,
	/// Discord handle.
	pub discord: Text<32>,
}

impl IdentityInfo {
	fn fields(&self) -> BitFlags<IdentityField> {
		let mut fields = BitFlags::<IdentityField>::empty();
		for (field, filled) in [
			(IdentityField::Display, !self.display.is_empty()),
			(IdentityField::Avatar, !self.avatar.is_empty()),
			(IdentityField::About, !self.about.is_empty()),
			(IdentityField::Web, !self.web.is_empty()),
			(IdentityField::Email, !self.email.is_empty()),
			(IdentityField::Github, !self.github.is_empty()),
			(IdentityField::Matrix, !self.matrix.is_empty()),
			(IdentityField::X, !self.x.is_empty()),
			(IdentityField::Telegram, !self.telegram.is_empty()),
			(IdentityField::Discord, !self.discord.is_empty()),
		] {
			if filled {
				fields.insert(field);
			}
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
		fn filled<const N: u32>() -> Text<N> {
			alloc::vec![b'x'; N as usize].try_into().expect("N bytes fit a bound of N")
		}

		IdentityInfo {
			display: filled(),
			avatar: filled(),
			about: filled(),
			web: filled(),
			email: filled(),
			github: filled(),
			matrix: filled(),
			x: filled(),
			telegram: filled(),
			discord: filled(),
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn all_fields() -> Self::FieldsIdentifier {
		IdentityField::all().bits()
	}
}
