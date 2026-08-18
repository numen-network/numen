//! OpenGov configuration. Token holders steer treasury spends and bounty
//! approvals through tiered spender tracks, each capping the amount its
//! referenda can release. Runtime level calls have no referendum track.

use crate::{
	AccountId, Balance, Balances, BlockNumber, Preimage, Referenda, Runtime, RuntimeCall,
	RuntimeEvent, RuntimeOrigin, Scheduler, System, Treasury, DAYS, HOURS, UNIT,
};
use alloc::borrow::Cow;
use frame_support::{
	parameter_types,
	traits::{AsEnsureOriginWithArg, ConstU32, Contains, EitherOf, EnsureOrigin},
};
use frame_system::{EnsureSigned, RawOrigin};
use pallet_identity::{Data, Judgement};
use pallet_referenda::{Curve, Track, TrackInfo};
use sp_runtime::{str_array as s, FixedI64};

pub use pallet_custom_origins::{BigSpender, MediumSpender, SmallSpender};

#[frame_support::pallet]
pub mod pallet_custom_origins {
	use crate::{Balance, UNIT};
	use frame_support::pallet_prelude::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[derive(
		PartialEq, Eq, Clone, MaxEncodedLen, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug,
	)]
	#[pallet::origin]
	pub enum Origin {
		/// Treasury spends and bounty approvals up to the small tier cap.
		SmallSpender,
		/// Treasury spends and bounty approvals up to the medium tier cap.
		MediumSpender,
		/// Treasury spends and bounty approvals up to the big tier cap.
		BigSpender,
	}

	macro_rules! decl_ensure {
		(
			$vis:vis type $name:ident: EnsureOrigin<Success = $success_type:ty> {
				$( $item:ident = $success:expr, )*
			}
		) => {
			$vis struct $name;
			impl<O: OriginTrait + From<Origin>> EnsureOrigin<O> for $name
			where
				for<'a> &'a O::PalletsOrigin: TryInto<&'a Origin>,
			{
				type Success = $success_type;
				fn try_origin(o: O) -> Result<Self::Success, O> {
					match o.caller().try_into() {
						$(
							Ok(Origin::$item) => return Ok($success),
						)*
						_ => (),
					}
					Err(o)
				}
				#[cfg(feature = "runtime-benchmarks")]
				fn try_successful_origin() -> Result<O, ()> {
					let _result: Result<O, ()> = Err(());
					$(
						let _result: Result<O, ()> = Ok(O::from(Origin::$item));
					)*
					_result
				}
			}
		};
	}

	decl_ensure! {
		pub type SmallSpender: EnsureOrigin<Success = Balance> {
			SmallSpender = 100_000 * UNIT,
		}
	}

	decl_ensure! {
		pub type MediumSpender: EnsureOrigin<Success = Balance> {
			MediumSpender = 1_000_000 * UNIT,
		}
	}

	decl_ensure! {
		pub type BigSpender: EnsureOrigin<Success = Balance> {
			BigSpender = 5_000_000 * UNIT,
		}
	}
}

const fn percent(x: i32) -> FixedI64 {
	FixedI64::from_rational(x as u128, 100)
}

const APP_SMALL: Curve = Curve::make_linear(7, 7, percent(50), percent(100));
const SUP_SMALL: Curve = Curve::make_linear(7, 7, percent(0), percent(50));
const APP_MEDIUM: Curve = Curve::make_linear(14, 14, percent(60), percent(100));
const SUP_MEDIUM: Curve = Curve::make_linear(14, 14, percent(2), percent(50));
const APP_BIG: Curve = Curve::make_linear(28, 28, percent(70), percent(100));
const SUP_BIG: Curve = Curve::make_linear(28, 28, percent(5), percent(50));

const TRACKS_DATA: [Track<u16, Balance, BlockNumber>; 3] = [
	Track {
		id: 0,
		info: TrackInfo {
			name: s("small_spender"),
			max_deciding: 20,
			decision_deposit: 100 * UNIT,
			prepare_period: HOURS,
			decision_period: 7 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: DAYS,
			min_approval: APP_SMALL,
			min_support: SUP_SMALL,
		},
	},
	Track {
		id: 1,
		info: TrackInfo {
			name: s("medium_spender"),
			max_deciding: 10,
			decision_deposit: 5_000 * UNIT,
			prepare_period: HOURS,
			decision_period: 14 * DAYS,
			confirm_period: 3 * DAYS,
			min_enactment_period: 3 * DAYS,
			min_approval: APP_MEDIUM,
			min_support: SUP_MEDIUM,
		},
	},
	Track {
		id: 2,
		info: TrackInfo {
			name: s("big_spender"),
			max_deciding: 4,
			decision_deposit: 100_000 * UNIT,
			prepare_period: HOURS,
			decision_period: 28 * DAYS,
			confirm_period: 7 * DAYS,
			min_enactment_period: 7 * DAYS,
			min_approval: APP_BIG,
			min_support: SUP_BIG,
		},
	},
];

pub struct TracksInfo;
impl pallet_referenda::TracksInfo<Balance, BlockNumber> for TracksInfo {
	type Id = u16;
	type RuntimeOrigin = <RuntimeOrigin as frame_support::traits::OriginTrait>::PalletsOrigin;

	fn tracks() -> impl Iterator<Item = Cow<'static, Track<Self::Id, Balance, BlockNumber>>> {
		TRACKS_DATA.iter().map(Cow::Borrowed)
	}

	fn track_for(id: &Self::RuntimeOrigin) -> Result<Self::Id, ()> {
		if let Ok(custom) = pallet_custom_origins::Origin::try_from(id.clone()) {
			match custom {
				pallet_custom_origins::Origin::SmallSpender => Ok(0),
				pallet_custom_origins::Origin::MediumSpender => Ok(1),
				pallet_custom_origins::Origin::BigSpender => Ok(2),
			}
		} else {
			Err(())
		}
	}
}

parameter_types! {
	pub const VoteLockingPeriod: BlockNumber = 7 * DAYS;
	pub const AlarmInterval: BlockNumber = 1;
	pub const SubmissionDeposit: Balance = 100 * UNIT;
	pub const UndecidingTimeout: BlockNumber = 14 * DAYS;
}

impl pallet_conviction_voting::Config for Runtime {
	type WeightInfo = crate::weights::pallet_conviction_voting::WeightInfo<Runtime>;
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type VoteLockingPeriod = VoteLockingPeriod;
	type MaxVotes = ConstU32<512>;
	type MaxTurnout =
		frame_support::traits::tokens::currency::ActiveIssuanceOf<Balances, AccountId>;
	type Polls = Referenda;
	type BlockNumberProvider = System;
	type VotingHooks = ();
}

impl pallet_custom_origins::Config for Runtime {}

/// Treasury and bounty spends accept any spender tier, each capped at its tier
/// amount.
pub type TreasurySpender = EitherOf<SmallSpender, EitherOf<MediumSpender, BigSpender>>;

/// Qualified identity standard shared by every entry point that gates on
/// identity. An account qualifies when a registrar judged its identity
/// Reasonable or KnownGood and the identity carries at least one social
/// channel among x, telegram and discord registered as plaintext. Hashed
/// channel commitments do not count since the point is public attribution.
/// A sub account qualifies through its parent since the SuperOf link keeps
/// the owner traceable.
pub struct QualifiedIdentity;

fn plaintext(data: &Data) -> bool {
	matches!(data, Data::Raw(bytes) if !bytes.is_empty())
}

impl QualifiedIdentity {
	fn account_qualifies(who: &AccountId) -> bool {
		pallet_identity::IdentityOf::<Runtime>::get(who).is_some_and(|registration| {
			let judged = registration
				.judgements
				.iter()
				.any(|(_, judgement)| matches!(judgement, Judgement::Reasonable | Judgement::KnownGood));
			let info = registration.info;
			let channel =
				plaintext(&info.x) || plaintext(&info.telegram) || plaintext(&info.discord);
			judged && channel
		})
	}
}

impl Contains<AccountId> for QualifiedIdentity {
	fn contains(who: &AccountId) -> bool {
		Self::account_qualifies(who)
			|| pallet_identity::SuperOf::<Runtime>::get(who)
				.is_some_and(|(parent, _)| Self::account_qualifies(&parent))
	}
}

/// Referendum submission is open to any account passing [`QualifiedIdentity`].
pub struct EnsureQualifiedIdentity;

impl EnsureOrigin<RuntimeOrigin> for EnsureQualifiedIdentity {
	type Success = AccountId;

	fn try_origin(o: RuntimeOrigin) -> Result<Self::Success, RuntimeOrigin> {
		let who = EnsureSigned::<AccountId>::try_origin(o)?;
		if QualifiedIdentity::contains(&who) {
			Ok(who)
		} else {
			Err(RawOrigin::Signed(who).into())
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin() -> Result<RuntimeOrigin, ()> {
		let who: AccountId = frame_benchmarking::whitelisted_caller();
		qualify_identity(&who);
		Ok(RawOrigin::Signed(who).into())
	}
}

/// Write a judged identity carrying a plaintext social channel straight into
/// storage so `who` passes [`QualifiedIdentity`]. Benchmarks cannot run a
/// registrar, yet every gate built on the identity standard needs an account
/// that clears it.
#[cfg(feature = "runtime-benchmarks")]
pub fn qualify_identity(who: &AccountId) {
	let info = crate::identity_info::IdentityInfo {
		x: Data::Raw(b"@bench".to_vec().try_into().expect("handle fits the field bound")),
		..Default::default()
	};
	let registration = pallet_identity::Registration {
		judgements: alloc::vec![(0, Judgement::Reasonable)]
			.try_into()
			.expect("one judgement fits MaxRegistrars"),
		deposit: 0,
		info,
	};
	pallet_identity::IdentityOf::<Runtime>::insert(who, registration);
}

impl pallet_referenda::Config for Runtime {
	type WeightInfo = pallet_referenda::weights::SubstrateWeight<Runtime>;
	type RuntimeCall = RuntimeCall;
	type RuntimeEvent = RuntimeEvent;
	type Scheduler = Scheduler;
	type Currency = Balances;
	type SubmitOrigin = AsEnsureOriginWithArg<EnsureQualifiedIdentity>;
	type CancelOrigin = pallet_prime::EnsurePrime<Runtime>;
	type KillOrigin = pallet_prime::EnsurePrime<Runtime>;
	type Slash = Treasury;
	type Votes = pallet_conviction_voting::VotesOf<Runtime>;
	type Tally = pallet_conviction_voting::TallyOf<Runtime>;
	type SubmissionDeposit = SubmissionDeposit;
	type MaxQueued = ConstU32<100>;
	type UndecidingTimeout = UndecidingTimeout;
	type AlarmInterval = AlarmInterval;
	type Tracks = TracksInfo;
	type Preimages = Preimage;
	type BlockNumberProvider = System;
}
