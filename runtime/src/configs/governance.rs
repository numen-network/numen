//! OpenGov configuration. Token holders steer the chain through tracks, each
//! naming the origin its referenda dispatch under and the bar they clear to
//! get there. Spender tracks also cap the amount a referendum can release.

use crate::{
	AccountId, Balance, Balances, BlockNumber, Preimage, Referenda, Runtime, RuntimeCall,
	RuntimeEvent, RuntimeOrigin, Scheduler, System, Treasury, DAYS, HOURS, MINUTES, UNIT,
};
use alloc::borrow::Cow;
use frame_support::{
	parameter_types,
	traits::{
		ConstU32, Contains, EitherOf, EitherOfDiverse, EnsureOrigin, EnsureOriginWithArg,
		OriginTrait,
	},
};
use frame_system::{EnsureSigned, RawOrigin};
use pallet_identity::Judgement;
use pallet_referenda::{Curve, Track, TrackInfo};
use sp_runtime::{str_array as s, FixedI64};

pub use pallet_custom_origins::{
	BigSpender, IdentityAdminOrigin, MediumSpender, ReferendumCanceller, ReferendumKiller,
	RuntimeUpgrade, SmallSpender,
};

/// The origin a referendum carries once it dispatches, which is also the key a
/// track is found by.
type PalletsOrigin = <RuntimeOrigin as OriginTrait>::PalletsOrigin;

#[frame_support::pallet]
pub mod pallet_custom_origins {
	use crate::{Balance, UNIT};
	use frame_support::pallet_prelude::*;

	pub const SMALL_SPENDER_CAP: Balance = 200_000 * UNIT;
	pub const MEDIUM_SPENDER_CAP: Balance = 1_000_000 * UNIT;
	pub const BIG_SPENDER_CAP: Balance = 10_000_000 * UNIT;

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
		/// Runtime code replacement.
		RuntimeUpgrade,
		/// A direction for the network to take, carrying no call of its own.
		WishForChange,
		/// Cancels a referendum, returning both of its deposits.
		ReferendumCanceller,
		/// Kills a referendum and slashes its decision deposit.
		ReferendumKiller,
		/// Appoints and retires identity registrars and username authorities.
		IdentityAdmin,
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
			SmallSpender = SMALL_SPENDER_CAP,
		}
	}

	decl_ensure! {
		pub type MediumSpender: EnsureOrigin<Success = Balance> {
			MediumSpender = MEDIUM_SPENDER_CAP,
		}
	}

	decl_ensure! {
		pub type BigSpender: EnsureOrigin<Success = Balance> {
			BigSpender = BIG_SPENDER_CAP,
		}
	}

	decl_ensure! {
		pub type RuntimeUpgrade: EnsureOrigin<Success = ()> {
			RuntimeUpgrade = (),
		}
	}

	decl_ensure! {
		pub type ReferendumCanceller: EnsureOrigin<Success = ()> {
			ReferendumCanceller = (),
		}
	}

	decl_ensure! {
		pub type ReferendumKiller: EnsureOrigin<Success = ()> {
			ReferendumKiller = (),
		}
	}

	decl_ensure! {
		pub type IdentityAdminOrigin: EnsureOrigin<Success = ()> {
			IdentityAdmin = (),
		}
	}

	#[pallet::extra_constants]
	impl<T: Config> Pallet<T> {
		#[pallet::constant_name(SpendCaps)]
		fn spend_caps() -> alloc::vec::Vec<(u16, Origin, Balance)> {
			alloc::vec![
				(30, Origin::SmallSpender, SMALL_SPENDER_CAP),
				(31, Origin::MediumSpender, MEDIUM_SPENDER_CAP),
				(32, Origin::BigSpender, BIG_SPENDER_CAP),
			]
		}

		#[pallet::constant_name(PreimageBaseDeposit)]
		fn preimage_base_deposit() -> Balance {
			crate::configs::PreimageBaseDeposit::get()
		}

		#[pallet::constant_name(PreimageByteDeposit)]
		fn preimage_byte_deposit() -> Balance {
			crate::configs::PreimageByteDeposit::get()
		}
	}
}

const fn percent(x: i32) -> FixedI64 {
	FixedI64::from_rational(x as u128, 100)
}

const fn per_mille(x: i32) -> FixedI64 {
	FixedI64::from_rational(x as u128, 1000)
}

const fn per_myriad(x: i32) -> FixedI64 {
	FixedI64::from_rational(x as u128, 10000)
}

// Approval opens at unanimity and eases to a bare majority. Support unwinds
// from half the supply across the decision period.
const APP_ROOT: Curve = Curve::make_reciprocal(4, 28, percent(80), percent(50), percent(100));
const SUP_ROOT: Curve = Curve::make_linear(28, 28, percent(0), percent(50));
const APP_WISH_FOR_CHANGE: Curve =
	Curve::make_reciprocal(4, 28, percent(80), percent(50), percent(100));
const SUP_WISH_FOR_CHANGE: Curve = Curve::make_linear(28, 28, percent(0), percent(50));

// Only prime opens an upgrade referendum, so support is there to let the rest
// of the chain object rather than to prove a quorum. It falls to 1% in a day.
const APP_RUNTIME_UPGRADE: Curve =
	Curve::make_reciprocal(4, 28, percent(80), percent(50), percent(100));
const SUP_RUNTIME_UPGRADE: Curve =
	Curve::make_reciprocal(1, 28, percent(1), percent(0), percent(50));

// Neither seat this track hands out is urgent, so support opens at half the
// supply and takes twelve days to reach 2%. It bottoms out at nothing,
// leaving the confirm period to set the bar.
const APP_IDENTITY_ADMIN: Curve =
	Curve::make_reciprocal(4, 28, percent(80), percent(50), percent(100));
const SUP_IDENTITY_ADMIN: Curve =
	Curve::make_reciprocal(12, 28, percent(2), percent(0), percent(50));

// Cancelling has to land before its target does, so it decides in seven days
// where every other track takes 28. Killing slashes the decision deposit, so
// it spends all 28. Support borrows the big spender shape and falls to
// nothing, leaving the confirm period to set the bar.
const APP_REFERENDUM_CANCELLER: Curve = Curve::make_linear(28, 28, percent(50), percent(100));
const SUP_REFERENDUM_CANCELLER: Curve =
	Curve::make_reciprocal(12, 28, percent(2), percent(0), percent(50));
const APP_REFERENDUM_KILLER: Curve = Curve::make_linear(28, 28, percent(50), percent(100));
const SUP_REFERENDUM_KILLER: Curve =
	Curve::make_reciprocal(12, 28, percent(2), percent(0), percent(50));

// Approval falls to a bare majority, taking longer the more a track can spend,
// and the support floor rises with the tier.
const APP_SMALL_SPENDER: Curve = Curve::make_linear(7, 28, percent(50), percent(100));
const SUP_SMALL_SPENDER: Curve =
	Curve::make_reciprocal(12, 28, percent(1), per_mille(5), percent(50));
const APP_MEDIUM_SPENDER: Curve = Curve::make_linear(14, 28, percent(50), percent(100));
const SUP_MEDIUM_SPENDER: Curve =
	Curve::make_reciprocal(12, 28, per_mille(15), per_myriad(75), percent(50));
const APP_BIG_SPENDER: Curve = Curve::make_linear(28, 28, percent(50), percent(100));
const SUP_BIG_SPENDER: Curve =
	Curve::make_reciprocal(12, 28, percent(2), percent(1), percent(50));

const TRACKS_DATA: [Track<u16, Balance, BlockNumber>; 9] = [
	Track {
		id: 0,
		info: TrackInfo {
			name: s("root"),
			max_deciding: 1,
			decision_deposit: 100_000 * UNIT,
			prepare_period: DAYS,
			decision_period: 28 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: DAYS,
			min_approval: APP_ROOT,
			min_support: SUP_ROOT,
		},
	},
	Track {
		id: 1,
		info: TrackInfo {
			name: s("runtime_upgrade"),
			max_deciding: 1,
			decision_deposit: 100 * UNIT,
			prepare_period: 10 * MINUTES,
			decision_period: 28 * DAYS,
			confirm_period: 10 * MINUTES,
			min_enactment_period: 10 * MINUTES,
			min_approval: APP_RUNTIME_UPGRADE,
			min_support: SUP_RUNTIME_UPGRADE,
		},
	},
	Track {
		id: 2,
		info: TrackInfo {
			name: s("wish_for_change"),
			max_deciding: 10,
			decision_deposit: 1_000 * UNIT,
			prepare_period: 2 * HOURS,
			decision_period: 28 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: 10 * MINUTES,
			min_approval: APP_WISH_FOR_CHANGE,
			min_support: SUP_WISH_FOR_CHANGE,
		},
	},
	Track {
		id: 10,
		info: TrackInfo {
			name: s("identity_admin"),
			max_deciding: 10,
			decision_deposit: 1_000 * UNIT,
			prepare_period: 2 * HOURS,
			decision_period: 28 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: 10 * MINUTES,
			min_approval: APP_IDENTITY_ADMIN,
			min_support: SUP_IDENTITY_ADMIN,
		},
	},
	Track {
		id: 20,
		info: TrackInfo {
			name: s("referendum_canceller"),
			max_deciding: 1_000,
			decision_deposit: 1_000 * UNIT,
			prepare_period: 2 * HOURS,
			decision_period: 7 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: 10 * MINUTES,
			min_approval: APP_REFERENDUM_CANCELLER,
			min_support: SUP_REFERENDUM_CANCELLER,
		},
	},
	Track {
		id: 21,
		info: TrackInfo {
			name: s("referendum_killer"),
			max_deciding: 1_000,
			decision_deposit: 10_000 * UNIT,
			prepare_period: 2 * HOURS,
			decision_period: 28 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: 10 * MINUTES,
			min_approval: APP_REFERENDUM_KILLER,
			min_support: SUP_REFERENDUM_KILLER,
		},
	},
	Track {
		id: 30,
		info: TrackInfo {
			name: s("small_spender"),
			max_deciding: 100,
			decision_deposit: 100 * UNIT,
			prepare_period: 4 * HOURS,
			decision_period: 28 * DAYS,
			confirm_period: DAYS,
			min_enactment_period: DAYS,
			min_approval: APP_SMALL_SPENDER,
			min_support: SUP_SMALL_SPENDER,
		},
	},
	Track {
		id: 31,
		info: TrackInfo {
			name: s("medium_spender"),
			max_deciding: 20,
			decision_deposit: 200 * UNIT,
			prepare_period: 4 * HOURS,
			decision_period: 28 * DAYS,
			confirm_period: 3 * DAYS,
			min_enactment_period: DAYS,
			min_approval: APP_MEDIUM_SPENDER,
			min_support: SUP_MEDIUM_SPENDER,
		},
	},
	Track {
		id: 32,
		info: TrackInfo {
			name: s("big_spender"),
			max_deciding: 2,
			decision_deposit: 1_000 * UNIT,
			prepare_period: 4 * HOURS,
			decision_period: 28 * DAYS,
			confirm_period: 7 * DAYS,
			min_enactment_period: DAYS,
			min_approval: APP_BIG_SPENDER,
			min_support: SUP_BIG_SPENDER,
		},
	},
];

pub struct TracksInfo;
impl pallet_referenda::TracksInfo<Balance, BlockNumber> for TracksInfo {
	type Id = u16;
	type RuntimeOrigin = PalletsOrigin;

	fn tracks() -> impl Iterator<Item = Cow<'static, Track<Self::Id, Balance, BlockNumber>>> {
		TRACKS_DATA.iter().map(Cow::Borrowed)
	}

	fn track_for(id: &Self::RuntimeOrigin) -> Result<Self::Id, ()> {
		use pallet_custom_origins::Origin;

		if let Ok(RawOrigin::Root) = RawOrigin::try_from(id.clone()) {
			return Ok(0);
		}
		match Origin::try_from(id.clone()).map_err(|_| ())? {
			Origin::RuntimeUpgrade => Ok(1),
			Origin::WishForChange => Ok(2),
			Origin::IdentityAdmin => Ok(10),
			Origin::ReferendumCanceller => Ok(20),
			Origin::ReferendumKiller => Ok(21),
			Origin::SmallSpender => Ok(30),
			Origin::MediumSpender => Ok(31),
			Origin::BigSpender => Ok(32),
		}
	}
}

parameter_types! {
	pub const VoteLockingPeriod: BlockNumber = 7 * DAYS;
	pub const AlarmInterval: BlockNumber = 1;
	pub const SubmissionDeposit: Balance = 5 * UNIT;
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
/// channel among x, telegram and discord. A sub account qualifies through its
/// parent since the SuperOf link keeps the owner traceable.
pub struct QualifiedIdentity;

impl QualifiedIdentity {
	fn account_qualifies(who: &AccountId) -> bool {
		pallet_identity::IdentityOf::<Runtime>::get(who).is_some_and(|registration| {
			let judged = registration
				.judgements
				.iter()
				.any(|(_, judgement)| matches!(judgement, Judgement::Reasonable | Judgement::KnownGood));
			let info = registration.info;
			let channel =
				!info.x.is_empty() || !info.telegram.is_empty() || !info.discord.is_empty();
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
		let who = <EnsureSigned<AccountId> as EnsureOrigin<_>>::try_origin(o)?;
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

/// Write a judged identity carrying a social channel straight into
/// storage so `who` passes [`QualifiedIdentity`]. Benchmarks cannot run a
/// registrar, yet every gate built on the identity standard needs an account
/// that clears it.
#[cfg(feature = "runtime-benchmarks")]
pub fn qualify_identity(who: &AccountId) {
	let info = crate::identity_info::IdentityInfo {
		x: b"bench".to_vec().try_into().expect("handle fits the field bound"),
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

/// Referendum submission is open to any account passing [`QualifiedIdentity`].
/// The upgrade track is prime's alone, since the code on offer is its own.
pub struct EnsureSubmitter;

impl EnsureOriginWithArg<RuntimeOrigin, PalletsOrigin> for EnsureSubmitter {
	type Success = AccountId;

	fn try_origin(o: RuntimeOrigin, track: &PalletsOrigin) -> Result<Self::Success, RuntimeOrigin> {
		match pallet_custom_origins::Origin::try_from(track.clone()) {
			Ok(pallet_custom_origins::Origin::RuntimeUpgrade) => {
				pallet_prime::EnsurePrime::<Runtime>::try_origin(o)
			},
			_ => EnsureQualifiedIdentity::try_origin(o),
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin(track: &PalletsOrigin) -> Result<RuntimeOrigin, ()> {
		match pallet_custom_origins::Origin::try_from(track.clone()) {
			Ok(pallet_custom_origins::Origin::RuntimeUpgrade) => {
				pallet_prime::EnsurePrime::<Runtime>::try_successful_origin()
			},
			_ => EnsureQualifiedIdentity::try_successful_origin(),
		}
	}
}

impl pallet_referenda::Config for Runtime {
	type WeightInfo = pallet_referenda::weights::SubstrateWeight<Runtime>;
	type RuntimeCall = RuntimeCall;
	type RuntimeEvent = RuntimeEvent;
	type Scheduler = Scheduler;
	type Currency = Balances;
	type SubmitOrigin = EnsureSubmitter;
	type CancelOrigin = EitherOfDiverse<pallet_prime::EnsurePrime<Runtime>, ReferendumCanceller>;
	type KillOrigin = EitherOfDiverse<pallet_prime::EnsurePrime<Runtime>, ReferendumKiller>;
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
