//! Hands the EVM state Frontier left on chain to pallet-revive.
//!
//! Contracts keep their address, code, storage, balance and nonce. They take
//! the shape pallet-revive gives contracts Root deployed, with no code or
//! storage deposits. Later refunds of that storage therefore come out at zero.
//! The Frontier accounts of contracts and known H160s move to the accounts
//! pallet-revive resolves those H160s to. The last 256 Ethereum block hashes
//! carry over, which keeps `BLOCKHASH` and the next block's parent linked.
//!
//! Nothing here panics. A contract that fails to move stays in Frontier's
//! storage and the log names it. Everything else goes ahead.

use alloc::vec::Vec;
use codec::{Decode, Encode};
use frame_support::{
	storage::{types::ValueQuery, unhashed, with_storage_layer},
	storage_alias,
	traits::{
		fungible::{Mutate, Unbalanced},
		OnRuntimeUpgrade,
	},
	weights::Weight,
	Blake2_128Concat, Identity, StorageHasher, Twox128, Twox64Concat,
};
use hex_literal::hex;
use pallet_revive::{
	AccountId32Mapper, AccountInfo, AddressMapper, BytecodeType, ContractBlob, ContractInfo,
	Deposit, ExecConfig, Executable, Key, TransactionLimits, TransactionMeter, H160, H256, U256,
};
use sp_runtime::{traits::Zero, DispatchError};

use crate::{
	balances_erc20, AccountId, Balance, Balances, BlockNumber, EvmClaims, Revive, Runtime,
	System,
};

const LOG_TARGET: &str = "runtime::frontier-migration";

const ETHEREUM_SCHEMA: &[u8] = b":ethereum_schema";

/// The H160s outside the contract set that appeared in EVM transactions and
/// logs up to block 443,501. A log topic counts only when its H160 has a
/// Frontier account. Their Frontier accounts move in the upgrade block, and an
/// H160 missing here can still claim.
const KNOWN_ADDRESSES: [[u8; 20]; 21] = [
	hex!("0000000000000000000000000000000000000001"),
	hex!("0000000000000000000000000000000000000802"),
	hex!("009cbad5058e4dec314ab374379e96d8ac3faafc"),
	hex!("0db8300836bd652c361c4158a8e69167cf33571f"),
	hex!("11d88cf4a500cd9e5184ca39a7ea5031d935a359"),
	hex!("1484c2834d1aa025d207f56fd299613684b49ec6"),
	hex!("1c0d1202d6b7983fdc6efad78298eff2b22a4e31"),
	hex!("24378e467264744246c12e1a8b003af97c1393cb"),
	hex!("2e4c21d1da532da62015c697f86b2f3ca297464a"),
	hex!("478c47d215bbfc43f25fab48b4100bb1bbe1a6dc"),
	hex!("5fdfd4d4f6e81424e3ab50d0848d4f0a2a83d86e"),
	hex!("6b1f4f97dca2909cac79564f7a9625c6427676db"),
	hex!("85979a3a0ae53eb4a439aa5013d215e4f6dfada8"),
	hex!("883fe0af53146b30d0166d7077025a4963deaefb"),
	hex!("946ad5a59b3bd16c3d443535a5b7fc07b56f5423"),
	hex!("a8f6efc25896c24ac6c9441f9f693c14517aa818"),
	hex!("b9cb2b8501933243a4ba5ad0f5fb3a256c8669fb"),
	hex!("bce52032cf2d4d11abffae6a1ba94c5d3e8cf9dc"),
	hex!("c6a7d1f198b9133468933247502c6d9025d32abd"),
	hex!("dee7fca01ef17dbf22d9a5dc02c877354decaa46"),
	hex!("f5ff20b442ea73f762361246fa782d0c8c9eb21b"),
];

/// Frontier's storage, read through aliases now that its pallets are gone.
pub mod frontier_storage {
	use super::*;

	#[storage_alias]
	pub type AccountCodes = StorageMap<EVM, Blake2_128Concat, H160, Vec<u8>>;

	#[storage_alias]
	pub type AccountCodesMetadata = StorageMap<EVM, Blake2_128Concat, H160, (u64, H256)>;

	#[storage_alias]
	pub type AccountStorages =
		StorageDoubleMap<EVM, Blake2_128Concat, H160, Blake2_128Concat, H256, H256, ValueQuery>;

	#[storage_alias]
	pub type BlockHash = StorageMap<Ethereum, Twox64Concat, U256, H256, ValueQuery>;
}

/// pallet-revive storage it keeps to itself.
pub mod revive_storage {
	use super::*;

	#[storage_alias]
	pub type BlockHash = StorageMap<Revive, Identity, BlockNumber, H256, ValueQuery>;

	/// pallet-revive's record of one stored code, field for field.
	#[derive(Encode, Decode)]
	pub struct CodeInfo {
		pub owner: AccountId,
		#[codec(compact)]
		pub deposit: Balance,
		#[codec(compact)]
		pub refcount: u64,
		pub code_len: u32,
		pub code_type: BytecodeType,
		pub behaviour_version: u32,
	}

	#[storage_alias]
	pub type CodeInfoOf = StorageMap<Revive, Identity, H256, CodeInfo>;
}

pub struct FrontierToRevive;

impl OnRuntimeUpgrade for FrontierToRevive {
	fn on_runtime_upgrade() -> Weight {
		let db = <Runtime as frame_system::Config>::DbWeight::get();
		if unhashed::get_raw(ETHEREUM_SCHEMA).is_none() {
			return db.reads(1);
		}
		let mut tally = Tally::default();

		copy_block_hashes(&mut tally);
		pin_signers(&mut tally);
		open_pallet_account(&mut tally);
		let contracts: Vec<(H160, Vec<u8>)> = frontier_storage::AccountCodes::iter().collect();
		tally.read(contracts.len());
		for (address, code) in contracts {
			if let Err(error) = with_storage_layer(|| move_contract(address, code, &mut tally)) {
				log::error!(target: LOG_TARGET, "contract {address:?} stays with Frontier: {error:?}");
			}
		}
		if let Err(error) = with_storage_layer(|| deploy_token_proxy(&mut tally)) {
			log::error!(target: LOG_TARGET, "no token proxy at {:?}: {error:?}", balances_erc20::ADDRESS);
		}
		for address in KNOWN_ADDRESSES.map(H160) {
			if let Err(error) = with_storage_layer(|| move_account(address, &mut tally)) {
				log::error!(target: LOG_TARGET, "account of {address:?} stays with Frontier: {error:?}");
			}
		}
		map_accounts(&mut tally);
		clear_frontier(&mut tally);

		db.reads_writes(tally.reads, tally.writes)
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade() -> Result<Vec<u8>, sp_runtime::TryRuntimeError> {
		Ok(checks::Snapshot::take().encode())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(state: Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
		let snapshot = checks::Snapshot::decode(&mut &state[..])
			.map_err(|_| "pre_upgrade state does not decode")?;
		snapshot.verify()
	}
}

/// Storage accesses made, which the migration weighs at the end.
#[derive(Default)]
struct Tally {
	reads: u64,
	writes: u64,
}

impl Tally {
	fn read(&mut self, count: usize) {
		self.reads = self.reads.saturating_add(count as u64);
	}

	fn write(&mut self, count: usize) {
		self.writes = self.writes.saturating_add(count as u64);
	}
}

/// The block numbers whose Ethereum hash `BLOCKHASH` still answers for.
fn recent_blocks() -> core::ops::RangeInclusive<BlockNumber> {
	let now = System::block_number();
	now.saturating_sub(256)..=now
}

fn copy_block_hashes(tally: &mut Tally) {
	for number in recent_blocks() {
		let key = U256::from(number);
		tally.read(1);
		if frontier_storage::BlockHash::contains_key(key) {
			revive_storage::BlockHash::insert(number, frontier_storage::BlockHash::get(key));
			tally.write(1);
		}
	}
}

/// Every account that signed under Frontier gets the provider that
/// `CheckNonceKeepSigned` gives on a first signature. Draining it can then no
/// longer reset its nonce.
fn pin_signers(tally: &mut Tally) {
	let mut scanned = 0;
	let signers: Vec<AccountId> = frame_system::Account::<Runtime>::iter()
		.inspect(|_| scanned += 1)
		.filter(|(_, info)| !info.nonce.is_zero())
		.map(|(who, _)| who)
		.collect();
	tally.read(scanned);
	for who in &signers {
		System::inc_providers(who);
	}
	tally.write(signers.len());
}

/// The account pallet-revive holds code deposits on, which its genesis opens.
fn open_pallet_account(tally: &mut Tally) {
	let account = Revive::account_id();
	tally.read(1);
	if System::account_exists(&account) {
		return;
	}
	let deposit = <Balances as frame_support::traits::fungible::Inspect<AccountId>>::minimum_balance();
	if <Balances as Mutate<AccountId>>::mint_into(&account, deposit).is_ok() {
		<Balances as Unbalanced<AccountId>>::deactivate(deposit);
	}
	tally.write(3);
}

/// Whether `code` is an EIP-7702 delegation, the marker `0xef0100` and the
/// address it delegates to.
fn is_delegation(code: &[u8]) -> bool {
	code.len() == 23 && code.starts_with(&hex!("ef0100"))
}

fn move_contract(address: H160, code: Vec<u8>, tally: &mut Tally) -> Result<(), DispatchError> {
	if is_delegation(&code) {
		log::info!(target: LOG_TARGET, "{address:?} carried an EIP-7702 delegation, moved as an account");
	} else {
		let storage: Vec<(H256, H256)> = frontier_storage::AccountStorages::iter_prefix(address)
			.filter(|(_, value): &(H256, H256)| !value.is_zero())
			.collect();
		tally.read(storage.len());
		install(address, code, &storage, tally)?;
	}
	EvmClaims::move_legacy(address)?;
	tally.read(10);
	tally.write(10);
	frontier_storage::AccountCodes::remove(address);
	frontier_storage::AccountCodesMetadata::remove(address);
	let removed = frontier_storage::AccountStorages::clear_prefix(address, u32::MAX, None);
	tally.write(2 + removed.unique as usize);
	Ok(())
}

fn deploy_token_proxy(tally: &mut Tally) -> Result<(), DispatchError> {
	tally.read(1);
	if AccountInfo::<Runtime>::is_contract(&balances_erc20::ADDRESS) {
		return Ok(());
	}
	install(balances_erc20::ADDRESS, balances_erc20::proxy_code(), &[], tally)
}

fn move_account(address: H160, tally: &mut Tally) -> Result<(), DispatchError> {
	tally.read(4);
	if !EvmClaims::claimable(&address) {
		return Ok(());
	}
	tally.read(10);
	tally.write(10);
	EvmClaims::move_legacy(address)
}

/// Puts `code` and `storage` at `address` the way pallet-revive holds a
/// contract Root deployed. The pallet account owns the code and no deposits
/// are held. The storage counters still count every slot as a 32 byte hashed
/// key and a 32 byte word.
fn install(
	address: H160,
	code: Vec<u8>,
	storage: &[(H256, H256)],
	tally: &mut Tally,
) -> Result<(), DispatchError> {
	let mut blob = ContractBlob::<Runtime>::from_evm_runtime_code_with_deposit(
		code,
		Revive::account_id(),
		Zero::zero(),
	)?;
	let code_hash = *blob.code_hash();
	let mut meter = TransactionMeter::<Runtime>::new(TransactionLimits::WeightAndDeposit {
		weight_limit: Weight::MAX,
		deposit_limit: Balance::MAX,
	})?;
	blob.store_code(&ExecConfig::new_substrate_tx(), &mut meter)?;
	revive_storage::CodeInfoOf::try_mutate(code_hash, |info| match info {
		Some(info) => {
			info.refcount = info.refcount.saturating_add(1);
			Ok(())
		},
		None => Err(DispatchError::Other("stored code has no code info")),
	})?;

	let mut info = ContractInfo::<Runtime>::new(&address, 0, code_hash)?;
	for (key, value) in storage {
		info.write(&Key::from_fixed(key.0), Some(value.0.to_vec()), None, false)?;
	}
	info.storage_items = storage.len() as u32;
	info.storage_bytes = 64 * storage.len() as u32;
	AccountInfo::<Runtime>::insert_contract(&address, info);

	let account = AccountId32Mapper::<Runtime>::to_fallback_account_id(&address);
	if !System::account_exists(&account) {
		<<Runtime as pallet_revive::Config>::Deposit as Deposit<Runtime>>::init_contract(&account)?;
	}
	System::inc_consumers(&account)?;
	if System::account_nonce(&account).is_zero() {
		System::inc_account_nonce(&account);
	}
	tally.read(8);
	tally.write(10 + storage.len());
	Ok(())
}

/// Gives every substrate account the H160 pallet-revive derives for it. Runs
/// after the Frontier accounts are gone, so none of those gets one.
fn map_accounts(tally: &mut Tally) {
	let accounts: Vec<AccountId> = frame_system::Account::<Runtime>::iter_keys()
		.filter(|who| {
			!AccountId32Mapper::<Runtime>::is_eth_derived(who) &&
				!AccountId32Mapper::<Runtime>::is_mapped(who)
		})
		.collect();
	tally.read(accounts.len());
	for who in &accounts {
		if let Err(error) = AccountId32Mapper::<Runtime>::map_no_deposit_unchecked(who) {
			log::error!(target: LOG_TARGET, "{who:?} stays unmapped: {error:?}");
		}
	}
	tally.write(accounts.len());
}

/// Drops Frontier's pallets. `EVM` keeps any contract that failed to move.
/// `EVMChainId` stays, since the chain id is read where it lies.
fn clear_frontier(tally: &mut Tally) {
	for pallet in [&b"Ethereum"[..], b"BaseFee"] {
		let removed = unhashed::clear_prefix(&Twox128::hash(pallet), None, None);
		tally.write(removed.unique as usize);
	}
	tally.read(1);
	if frontier_storage::AccountCodes::iter_keys().next().is_none() {
		let removed = unhashed::clear_prefix(&Twox128::hash(b"EVM"), None, None);
		tally.write(removed.unique as usize);
	}
	unhashed::kill(ETHEREUM_SCHEMA);
	tally.write(1);
}

#[cfg(feature = "try-runtime")]
mod checks {
	use super::*;
	use frame_support::{
		ensure,
		traits::{fungible::Inspect, Get},
	};
	use sp_runtime::TryRuntimeError;

	type Nonce = <Runtime as frame_system::Config>::Nonce;

	/// A contract with the EVM balance it is due afterwards and the nonce its
	/// Frontier account reached.
	#[derive(Encode, Decode)]
	struct Contract {
		address: H160,
		code: Vec<u8>,
		storage: Vec<(H256, H256)>,
		balance: U256,
		nonce: Nonce,
	}

	#[derive(Encode, Decode)]
	pub struct Snapshot {
		total_issuance: Balance,
		active_issuance: Balance,
		contracts: Vec<Contract>,
		/// Addresses that carried an EIP-7702 delegation and move as accounts.
		delegations: Vec<H160>,
		/// Address with the EVM balance it is due afterwards and the nonce its
		/// Frontier account reached.
		accounts: Vec<(H160, U256, Nonce)>,
		block_hashes: Vec<(BlockNumber, H256)>,
		/// Signers that stay where they are, with their providers.
		signers: Vec<(AccountId, u32)>,
		/// Existential deposits the migration mints.
		minted: Balance,
		chain_id: u64,
	}

	/// The EVM balance `address` is due once its Frontier account moved, and
	/// the nonce that account reached.
	fn due(address: &H160) -> (U256, Nonce) {
		let legacy = EvmClaims::legacy_account(address);
		let balance = Revive::evm_balance(address) + U256::from(Balances::balance(&legacy));
		(balance, System::account_nonce(&legacy))
	}

	impl Snapshot {
		pub fn take() -> Self {
			let ed = Balances::minimum_balance();
			let (delegations, contracts): (Vec<_>, Vec<_>) =
				frontier_storage::AccountCodes::iter().partition(|(_, code)| is_delegation(code));
			let delegations: Vec<H160> = delegations.into_iter().map(|(address, _)| address).collect();
			let contracts: Vec<_> = contracts
				.into_iter()
				.map(|(address, code)| {
					let storage = frontier_storage::AccountStorages::iter_prefix(address)
						.filter(|(_, value): &(H256, H256)| !value.is_zero())
						.collect();
					let (balance, nonce) = due(&address);
					Contract { address, code, storage, balance, nonce }
				})
				.collect();
			let accounts: Vec<_> = KNOWN_ADDRESSES
				.map(H160)
				.into_iter()
				.filter(|address| EvmClaims::claimable(address) && !delegations.contains(address))
				.chain(delegations.iter().copied())
				.map(|address| {
					let (balance, nonce) = due(&address);
					(address, balance, nonce)
				})
				.collect();
			let moved: Vec<AccountId> = contracts
				.iter()
				.map(|contract| EvmClaims::legacy_account(&contract.address))
				.chain(accounts.iter().map(|(address, ..)| EvmClaims::legacy_account(address)))
				.collect();
			let signers = frame_system::Account::<Runtime>::iter()
				.filter(|(who, info)| !info.nonce.is_zero() && !moved.contains(who))
				.map(|(who, info)| (who, info.providers))
				.collect();
			let fresh = |address: &H160| {
				Balances::total_balance(&AccountId32Mapper::<Runtime>::to_fallback_account_id(address))
					.is_zero()
			};
			let mints = contracts.iter().filter(|contract| fresh(&contract.address)).count() +
				accounts
					.iter()
					.filter(|(address, balance, _)| !balance.is_zero() && fresh(address))
					.count() +
				usize::from(fresh(&balances_erc20::ADDRESS)) +
				usize::from(!System::account_exists(&Revive::account_id()));
			let block_hashes = recent_blocks()
				.filter(|number| frontier_storage::BlockHash::contains_key(U256::from(*number)))
				.map(|number| (number, frontier_storage::BlockHash::get(U256::from(number))))
				.collect();
			Self {
				total_issuance: Balances::total_issuance(),
				active_issuance: Balances::active_issuance(),
				contracts,
				delegations,
				accounts,
				block_hashes,
				signers,
				minted: ed * mints as Balance,
				chain_id: crate::configs::evm::EvmChainId::get(),
			}
		}

		pub fn verify(self) -> Result<(), TryRuntimeError> {
			for Contract { address, code, storage, balance, nonce } in &self.contracts {
				ensure!(Revive::code(address) == *code, "contract code changed");
				let info = AccountInfo::<Runtime>::load_contract(address).ok_or("contract missing")?;
				ensure!(info.storage_items as usize == storage.len(), "storage item count");
				ensure!(info.storage_bytes as usize == 64 * storage.len(), "storage byte count");
				ensure!(info.total_deposit().is_zero(), "deposits on a moved contract");
				for (key, value) in storage {
					ensure!(
						matches!(Revive::get_storage(*address, key.0), Ok(Some(word)) if word == value.0),
						"storage slot changed"
					);
				}
				let account = AccountId32Mapper::<Runtime>::to_fallback_account_id(address);
				ensure!(Revive::evm_balance(address) == *balance, "contract balance");
				ensure!(System::account_nonce(&account) == (*nonce).max(1), "contract nonce");
				ensure!(frame_system::Account::<Runtime>::get(&account).consumers >= 1, "no consumer");
				let code_info = revive_storage::CodeInfoOf::get(info.code_hash).ok_or("code info")?;
				let users = self.contracts.iter().filter(|other| other.code == *code).count();
				ensure!(code_info.refcount as usize == users, "code refcount");
				ensure!(code_info.deposit.is_zero(), "code deposit");
			}
			for address in &self.delegations {
				ensure!(!AccountInfo::<Runtime>::is_contract(address), "delegation installed as a contract");
			}
			for (address, balance, nonce) in &self.accounts {
				ensure!(Revive::evm_balance(address) == *balance, "account balance");
				let account = AccountId32Mapper::<Runtime>::to_account_id(address);
				ensure!(System::account_nonce(&account) >= *nonce, "account nonce");
			}
			let moved = self
				.contracts
				.iter()
				.map(|contract| &contract.address)
				.chain(self.accounts.iter().map(|(address, ..)| address));
			for address in moved {
				ensure!(
					!frame_system::Account::<Runtime>::contains_key(EvmClaims::legacy_account(address)),
					"Frontier account left behind"
				);
			}
			ensure!(Revive::code(&balances_erc20::ADDRESS) == balances_erc20::proxy_code(), "token proxy");
			for (number, hash) in &self.block_hashes {
				ensure!(revive_storage::BlockHash::get(number) == *hash, "block hash");
			}
			for (who, providers) in &self.signers {
				ensure!(
					frame_system::Account::<Runtime>::get(who).providers == providers + 1,
					"signer not pinned once"
				);
			}
			for who in frame_system::Account::<Runtime>::iter_keys() {
				ensure!(
					AccountId32Mapper::<Runtime>::is_eth_derived(&who) ||
						AccountId32Mapper::<Runtime>::is_mapped(&who),
					"account left unmapped"
				);
			}
			ensure!(
				Balances::total_issuance() == self.total_issuance + self.minted,
				"issuance moved beyond the minted existential deposits"
			);
			ensure!(Balances::active_issuance() == self.active_issuance, "active issuance moved");
			ensure!(crate::configs::evm::EvmChainId::get() == self.chain_id, "chain id moved");
			for pallet in [&b"Ethereum"[..], b"BaseFee", b"EVM"] {
				ensure!(!unhashed::contains_prefixed_key(&Twox128::hash(pallet)), "Frontier storage left");
			}
			ensure!(unhashed::get_raw(ETHEREUM_SCHEMA).is_none(), "schema marker left");
			Ok(())
		}
	}
}
