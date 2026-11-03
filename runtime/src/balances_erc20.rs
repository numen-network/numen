//! The native token as an ERC20 at `0x…0802`, the address wallets, contracts
//! and signed permits already point at.
//!
//! Precompile addresses end in two zero bytes. The precompile therefore sits
//! at `0x…08020000`, behind a proxy contract at `0x…0802` that delegatecalls
//! into it. It serves the proxy alone, which makes the account that called the
//! proxy the one whose funds move.

use alloc::{string::String, vec::Vec};
use core::num::NonZero;
use frame_support::{
	storage::types::{StorageDoubleMap, StorageMap, ValueQuery},
	traits::{
		fungible::{Inspect, Mutate, Unbalanced},
		tokens::Preservation,
		StorageInstance,
	},
	weights::Weight,
	Blake2_128Concat,
};
use pallet_balances::WeightInfo as _;
use pallet_revive::precompiles::{
	alloy::{
		primitives::{IntoLogData, U256 as EvmU256},
		sol,
		sol_types::{Revert, SolCall},
	},
	AddressMapper, AddressMatcher, Error, Ext, Precompile, RuntimeCosts, H160, H256, U256,
};

use sp_runtime::traits::{Hash, Keccak256};

use crate::{AccountId, Balance, Balances, Runtime, System};

const SLOT: u16 = 0x0802;

const NAME: &str = "Numen";
const SYMBOL: &str = "NUMN";
const DECIMALS: u8 = 18;

/// The token address, where the proxy contract lives.
pub const ADDRESS: H160 = slot_address(18);

/// Where revive mounts the precompile.
pub const PRECOMPILE: H160 = slot_address(16);

const fn slot_address(at: usize) -> H160 {
	let mut address = [0u8; 20];
	let slot = SLOT.to_be_bytes();
	address[at] = slot[0];
	address[at + 1] = slot[1];
	H160(address)
}

sol! {
	interface IBalancesErc20 {
		function name() external view returns (string memory);
		function symbol() external view returns (string memory);
		function decimals() external view returns (uint8);
		function totalSupply() external view returns (uint256);
		function balanceOf(address owner) external view returns (uint256);
		function allowance(address owner, address spender) external view returns (uint256);
		function approve(address spender, uint256 value) external returns (bool);
		function transfer(address to, uint256 value) external returns (bool);
		function transferFrom(address from, address to, uint256 value) external returns (bool);
		function withdraw(bytes32 dest, uint256 value) external returns (bool);
		function deposit() external payable;
		function nonces(address owner) external view returns (uint256);
		function DOMAIN_SEPARATOR() external view returns (bytes32);
		function permit(
			address owner,
			address spender,
			uint256 value,
			uint256 deadline,
			uint8 v,
			bytes32 r,
			bytes32 s
		) external;

		event Transfer(address indexed from, address indexed to, uint256 value);
		event Approval(address indexed owner, address indexed spender, uint256 value);
		event Deposit(address indexed dst, uint256 wad);
		event Withdrawal(address indexed src, bytes32 indexed dest, uint256 value);
	}
}

use IBalancesErc20::{IBalancesErc20Calls as Call, IBalancesErc20Events as Event};

pub struct AllowancesPrefix;

impl StorageInstance for AllowancesPrefix {
	const STORAGE_PREFIX: &'static str = "Allowances";
	fn pallet_prefix() -> &'static str {
		"EvmBalancesErc20"
	}
}

/// `(owner, spender) -> allowance`.
pub type Allowances =
	StorageDoubleMap<AllowancesPrefix, Blake2_128Concat, H160, Blake2_128Concat, H160, U256, ValueQuery>;

pub struct NoncesPrefix;

impl StorageInstance for NoncesPrefix {
	const STORAGE_PREFIX: &'static str = "Nonces";
	fn pallet_prefix() -> &'static str {
		"EvmBalancesErc20"
	}
}

/// EIP-2612 permit nonce of each owner.
pub type Nonces = StorageMap<NoncesPrefix, Blake2_128Concat, H160, U256, ValueQuery>;

pub struct BalancesErc20;

impl Precompile for BalancesErc20 {
	type T = Runtime;
	type Interface = Call;
	const MATCHER: AddressMatcher = AddressMatcher::Fixed(NonZero::new(SLOT).unwrap());
	const HAS_CONTRACT_INFO: bool = false;

	fn call(
		_address: &[u8; 20],
		input: &Call,
		env: &mut impl Ext<T = Runtime>,
	) -> Result<Vec<u8>, Error> {
		if env.address() != ADDRESS {
			return Err(revert("ERC20: call the token at 0x0000000000000000000000000000000000000802"));
		}
		let writes = matches!(
			input,
			Call::approve(_)
				| Call::transfer(_)
				| Call::transferFrom(_)
				| Call::withdraw(_)
				| Call::deposit(_)
				| Call::permit(_)
		);
		if writes && env.is_read_only() {
			return Err(revert("Can't call non-static function in static context"));
		}
		if !matches!(input, Call::deposit(_)) && !env.value_transferred().is_zero() {
			return Err(revert("Function is not payable"));
		}

		match input {
			Call::name(_) => Ok(IBalancesErc20::nameCall::abi_encode_returns(&String::from(NAME))),
			Call::symbol(_) =>
				Ok(IBalancesErc20::symbolCall::abi_encode_returns(&String::from(SYMBOL))),
			Call::decimals(_) => Ok(IBalancesErc20::decimalsCall::abi_encode_returns(&DECIMALS)),
			Call::totalSupply(_) => total_supply(env),
			Call::balanceOf(call) => balance_of(call, env),
			Call::allowance(call) => allowance(call, env),
			Call::approve(call) => approve(call, env),
			Call::transfer(call) => transfer(call, env),
			Call::transferFrom(call) => transfer_from(call, env),
			Call::withdraw(call) => withdraw(call, env),
			Call::deposit(_) => deposit(env),
			Call::nonces(call) => nonces(call, env),
			Call::DOMAIN_SEPARATOR(_) => domain(env),
			Call::permit(call) => permit(call, env),
		}
	}
}

fn total_supply(env: &mut impl Ext<T = Runtime>) -> Result<Vec<u8>, Error> {
	env.charge(db_reads(1))?;
	let supply = EvmU256::from(<Balances as Inspect<AccountId>>::total_issuance());
	Ok(IBalancesErc20::totalSupplyCall::abi_encode_returns(&supply))
}

fn balance_of(
	call: &IBalancesErc20::balanceOfCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.frame_meter_mut().charge_weight_token(RuntimeCosts::BalanceOf)?;
	let balance = env.balance_of(&h160(call.owner));
	Ok(IBalancesErc20::balanceOfCall::abi_encode_returns(&to_evm(balance)))
}

fn allowance(
	call: &IBalancesErc20::allowanceCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(db_reads(1))?;
	let allowance = Allowances::get(h160(call.owner), h160(call.spender));
	Ok(IBalancesErc20::allowanceCall::abi_encode_returns(&to_evm(allowance)))
}

fn approve(
	call: &IBalancesErc20::approveCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(db_writes(1))?;
	let (_, owner) = caller(env)?;
	Allowances::insert(owner, h160(call.spender), from_evm(call.value));
	emit(
		env,
		Event::Approval(IBalancesErc20::Approval {
			owner: owner.0.into(),
			spender: call.spender,
			value: call.value,
		}),
	)?;
	Ok(IBalancesErc20::approveCall::abi_encode_returns(&true))
}

fn transfer(
	call: &IBalancesErc20::transferCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(transfer_weight())?;
	let (from_account, from) = caller(env)?;
	let to = account_of(env, h160(call.to))?;
	move_funds(&from_account, &to, call.value)?;
	emit(
		env,
		Event::Transfer(IBalancesErc20::Transfer {
			from: from.0.into(),
			to: call.to,
			value: call.value,
		}),
	)?;
	Ok(IBalancesErc20::transferCall::abi_encode_returns(&true))
}

fn transfer_from(
	call: &IBalancesErc20::transferFromCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(transfer_weight().saturating_add(db_reads(1)).saturating_add(db_writes(1)))?;
	let (_, spender) = caller(env)?;
	let from = h160(call.from);
	if from != spender {
		let allowance = Allowances::get(from, spender)
			.checked_sub(from_evm(call.value))
			.ok_or_else(|| revert("ERC20: insufficient allowance"))?;
		Allowances::insert(from, spender, allowance);
	}
	let from_account = account_of(env, from)?;
	let to = account_of(env, h160(call.to))?;
	move_funds(&from_account, &to, call.value)?;
	emit(
		env,
		Event::Transfer(IBalancesErc20::Transfer { from: call.from, to: call.to, value: call.value }),
	)?;
	Ok(IBalancesErc20::transferFromCall::abi_encode_returns(&true))
}

fn withdraw(
	call: &IBalancesErc20::withdrawCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(transfer_weight())?;
	let (from_account, from) = caller(env)?;
	move_funds(&from_account, &AccountId::from(call.dest.0), call.value)?;
	emit(
		env,
		Event::Withdrawal(IBalancesErc20::Withdrawal {
			src: from.0.into(),
			dest: call.dest,
			value: call.value,
		}),
	)?;
	Ok(IBalancesErc20::withdrawCall::abi_encode_returns(&true))
}

/// The native token and the ERC20 are one balance, so a deposit hands the
/// value straight back and only leaves the event.
fn deposit(env: &mut impl Ext<T = Runtime>) -> Result<Vec<u8>, Error> {
	env.charge(transfer_weight())?;
	let value = env.value_transferred();
	if value.is_zero() {
		return Err(revert("ERC20: cannot deposit zero"));
	}
	let (to_account, to) = caller(env)?;
	let amount = Balance::try_from(value).map_err(|_| revert("ERC20: amount overflow"))?;
	<Balances as Mutate<AccountId>>::transfer(
		env.account_id(),
		&to_account,
		amount,
		Preservation::Preserve,
	)
	.map_err(|_| revert("ERC20: deposit refund failed"))?;
	emit(env, Event::Deposit(IBalancesErc20::Deposit { dst: to.0.into(), wad: to_evm(value) }))?;
	Ok(Vec::new())
}

fn nonces(
	call: &IBalancesErc20::noncesCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(db_reads(1))?;
	let nonce = Nonces::get(h160(call.owner));
	Ok(IBalancesErc20::noncesCall::abi_encode_returns(&to_evm(nonce)))
}

fn domain(env: &mut impl Ext<T = Runtime>) -> Result<Vec<u8>, Error> {
	env.charge(db_reads(1))?;
	let separator = domain_separator(env)?;
	Ok(IBalancesErc20::DOMAIN_SEPARATORCall::abi_encode_returns(&separator.into()))
}

fn permit(
	call: &IBalancesErc20::permitCall,
	env: &mut impl Ext<T = Runtime>,
) -> Result<Vec<u8>, Error> {
	env.charge(db_reads(2).saturating_add(db_writes(2)))?;
	if env.now() > from_evm(call.deadline) {
		return Err(revert("Permit expired"));
	}
	let owner = h160(call.owner);
	let spender = h160(call.spender);
	let nonce = Nonces::get(owner);

	let mut permit = [0u8; 32 * 6];
	permit[..32].copy_from_slice(&keccak(
		env,
		b"Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
	)?);
	permit[32 + 12..64].copy_from_slice(owner.as_bytes());
	permit[64 + 12..96].copy_from_slice(spender.as_bytes());
	permit[96..128].copy_from_slice(&call.value.to_be_bytes::<32>());
	permit[128..160].copy_from_slice(&nonce.to_big_endian());
	permit[160..].copy_from_slice(&call.deadline.to_be_bytes::<32>());

	let mut digest = [0u8; 2 + 32 + 32];
	digest[..2].copy_from_slice(b"\x19\x01");
	digest[2..34].copy_from_slice(&domain_separator(env)?);
	digest[34..].copy_from_slice(&keccak(env, &permit)?);
	let digest = keccak(env, &digest)?;

	if signer(env, &digest, call.v, &call.r.0, &call.s.0)? != Some(owner) {
		return Err(revert("Invalid permit"));
	}
	Nonces::insert(owner, nonce.saturating_add(U256::one()));
	Allowances::insert(owner, spender, from_evm(call.value));
	emit(
		env,
		Event::Approval(IBalancesErc20::Approval {
			owner: call.owner,
			spender: call.spender,
			value: call.value,
		}),
	)?;
	Ok(Vec::new())
}

/// EIP-712 domain the permits are signed over.
fn domain_separator(env: &mut impl Ext<T = Runtime>) -> Result<[u8; 32], Error> {
	let mut domain = [0u8; 32 * 5];
	domain[..32].copy_from_slice(&keccak(
		env,
		b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
	)?);
	domain[32..64].copy_from_slice(&keccak(env, NAME.as_bytes())?);
	domain[64..96].copy_from_slice(&keccak(env, b"1")?);
	domain[96..128].copy_from_slice(&U256::from(env.chain_id()).to_big_endian());
	domain[128 + 12..].copy_from_slice(ADDRESS.as_bytes());
	keccak(env, &domain)
}

fn signer(
	env: &mut impl Ext<T = Runtime>,
	digest: &[u8; 32],
	v: u8,
	r: &[u8; 32],
	s: &[u8; 32],
) -> Result<Option<H160>, Error> {
	let recovery_id = match v {
		27 | 28 => v - 27,
		0 | 1 => v,
		_ => return Ok(None),
	};
	env.frame_meter_mut().charge_weight_token(RuntimeCosts::EcdsaRecovery)?;
	let mut signature = [0u8; 65];
	signature[..32].copy_from_slice(r);
	signature[32..64].copy_from_slice(s);
	signature[64] = recovery_id;
	let Ok(public) = env.ecdsa_recover(&signature, digest) else { return Ok(None) };
	env.frame_meter_mut().charge_weight_token(RuntimeCosts::EcdsaToEthAddress)?;
	Ok(env.ecdsa_to_eth_address(&public).ok().map(H160))
}

fn keccak(env: &mut impl Ext<T = Runtime>, data: &[u8]) -> Result<[u8; 32], Error> {
	env.frame_meter_mut()
		.charge_weight_token(RuntimeCosts::HashKeccak256(data.len() as u32))?;
	Ok(Keccak256::hash(data).0)
}

/// Moves `value` and keeps the existential deposit behind. An Ethereum account
/// can thus spend everything its balance shows yet never empties. An account
/// that does not exist yet first gets an inactive existential deposit, so it
/// receives `value` whole, as from a plain value transfer.
fn move_funds(from: &AccountId, to: &AccountId, value: EvmU256) -> Result<(), Error> {
	let amount = Balance::try_from(value).map_err(|_| revert("ERC20: amount overflow"))?;
	if amount == 0 {
		return Ok(());
	}
	if !System::account_exists(to) {
		let deposit = <Balances as Inspect<AccountId>>::minimum_balance();
		<Balances as Mutate<AccountId>>::mint_into(to, deposit)?;
		<Balances as Unbalanced<AccountId>>::deactivate(deposit);
	}
	<Balances as Mutate<AccountId>>::transfer(from, to, amount, Preservation::Preserve)
		.map_err(|_| revert("ERC20: transfer failed"))?;
	Ok(())
}

fn caller(env: &mut impl Ext<T = Runtime>) -> Result<(AccountId, H160), Error> {
	env.frame_meter_mut().charge_weight_token(RuntimeCosts::Caller)?;
	let caller = env.caller();
	let account = caller.account_id().map_err(|_| revert("ERC20: no account to act for"))?;
	let address = <Runtime as pallet_revive::Config>::AddressMapper::to_address(account);
	Ok((account.clone(), address))
}

fn account_of(env: &mut impl Ext<T = Runtime>, address: H160) -> Result<AccountId, Error> {
	env.frame_meter_mut().charge_weight_token(RuntimeCosts::ToAccountId)?;
	Ok(<Runtime as pallet_revive::Config>::AddressMapper::to_account_id(&address))
}

fn emit(env: &mut impl Ext<T = Runtime>, event: Event) -> Result<(), Error> {
	let (topics, data) = event.into_log_data().split();
	let topics: Vec<H256> = topics.into_iter().map(|topic| H256(topic.0)).collect();
	env.frame_meter_mut().charge_weight_token(RuntimeCosts::DepositEvent {
		num_topic: topics.len() as u32,
		len: data.len() as u32,
	})?;
	env.deposit_event(topics, data.to_vec());
	Ok(())
}

/// A transfer, with the issuance writes of a new account's deposit.
fn transfer_weight() -> Weight {
	<Runtime as pallet_balances::Config>::WeightInfo::transfer_keep_alive().saturating_add(db_writes(2))
}

fn db_reads(count: u64) -> Weight {
	<Runtime as frame_system::Config>::DbWeight::get().reads(count)
}

fn db_writes(count: u64) -> Weight {
	<Runtime as frame_system::Config>::DbWeight::get().writes(count)
}

fn revert(reason: &str) -> Error {
	Error::Revert(Revert { reason: reason.into() })
}

fn h160(address: pallet_revive::precompiles::alloy::primitives::Address) -> H160 {
	H160(address.into_array())
}

fn to_evm(value: U256) -> EvmU256 {
	EvmU256::from_be_bytes(value.to_big_endian())
}

fn from_evm(value: EvmU256) -> U256 {
	U256::from_big_endian(&value.to_be_bytes::<32>())
}

/// Code of the proxy at [`ADDRESS`]. Calls with a known selector pass through
/// to the precompile untouched. Anything else becomes `deposit()`, the way
/// value sent to a WETH9 contract is a deposit. Return data and reverts come
/// back as they are.
///
/// The code lives on chain once deployed, so a change here needs a migration
/// that rewrites the code at [`ADDRESS`].
pub fn proxy_code() -> Vec<u8> {
	use opcode::*;

	let mut code = Vec::from([PUSH1, 4, CALLDATASIZE, LT]);
	let to_deposit = jump(&mut code, JUMPI);
	code.extend([PUSH1, 0, CALLDATALOAD, PUSH1, 0xe0, SHR]);
	let to_forward: Vec<usize> = Call::SELECTORS
		.iter()
		.map(|selector| {
			code.extend([DUP1, PUSH4]);
			code.extend(selector);
			code.push(EQ);
			jump(&mut code, JUMPI)
		})
		.collect();

	land(&mut code, &[to_deposit]);
	code.push(PUSH4);
	code.extend(IBalancesErc20::depositCall::SELECTOR);
	code.extend([PUSH1, 0xe0, SHL, PUSH1, 0, MSTORE, PUSH1, 4]);
	let to_call = jump(&mut code, JUMP);

	land(&mut code, &to_forward);
	code.extend([CALLDATASIZE, PUSH1, 0, PUSH1, 0, CALLDATACOPY, CALLDATASIZE]);

	land(&mut code, &[to_call]);
	code.extend([PUSH1, 0, PUSH1, 0, DUP3, PUSH1, 0, PUSH4]);
	code.extend(&PRECOMPILE.as_bytes()[16..]);
	code.extend([GAS, DELEGATECALL, RETURNDATASIZE, PUSH1, 0, PUSH1, 0, RETURNDATACOPY]);
	let to_return = jump(&mut code, JUMPI);
	code.extend([RETURNDATASIZE, PUSH1, 0, REVERT]);

	land(&mut code, &[to_return]);
	code.extend([RETURNDATASIZE, PUSH1, 0, RETURN]);
	code
}

/// Appends a jump to a target [`land`] fills in later.
fn jump(code: &mut Vec<u8>, kind: u8) -> usize {
	code.extend([opcode::PUSH2, 0, 0, kind]);
	code.len() - 3
}

/// Places a jump target here and points `jumps` at it.
fn land(code: &mut Vec<u8>, jumps: &[usize]) {
	let target = (code.len() as u16).to_be_bytes();
	for &at in jumps {
		code[at..at + 2].copy_from_slice(&target);
	}
	code.push(opcode::JUMPDEST);
}

mod opcode {
	pub const LT: u8 = 0x10;
	pub const EQ: u8 = 0x14;
	pub const SHL: u8 = 0x1b;
	pub const SHR: u8 = 0x1c;
	pub const CALLDATALOAD: u8 = 0x35;
	pub const CALLDATASIZE: u8 = 0x36;
	pub const CALLDATACOPY: u8 = 0x37;
	pub const RETURNDATASIZE: u8 = 0x3d;
	pub const RETURNDATACOPY: u8 = 0x3e;
	pub const MSTORE: u8 = 0x52;
	pub const JUMP: u8 = 0x56;
	pub const JUMPI: u8 = 0x57;
	pub const GAS: u8 = 0x5a;
	pub const JUMPDEST: u8 = 0x5b;
	pub const PUSH1: u8 = 0x60;
	pub const PUSH2: u8 = 0x61;
	pub const PUSH4: u8 = 0x63;
	pub const DUP1: u8 = 0x80;
	pub const DUP3: u8 = 0x82;
	pub const RETURN: u8 = 0xf3;
	pub const DELEGATECALL: u8 = 0xf4;
	pub const REVERT: u8 = 0xfd;
}
