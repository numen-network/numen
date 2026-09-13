//! Externalities builder and EVM helpers shared by the runtime tests.

#![allow(dead_code)]

use numen_runtime::{AccountId, Precompiles, Runtime, System};
use pallet_evm::AddressMapping;
use sp_core::{H160, U256};
use sp_io::TestExternalities;
use sp_runtime::BuildStorage;

/// Empty genesis with the block number advanced to 1 so events are recorded.
pub fn new_test_ext() -> TestExternalities {
	let storage = frame_system::GenesisConfig::<Runtime>::default()
		.build_storage()
		.expect("system genesis builds");
	let mut ext = TestExternalities::from(storage);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

pub fn evm_account(addr: H160) -> AccountId {
	<Runtime as pallet_evm::Config>::AddressMapping::into_account_id(addr)
}

pub fn balances_erc20() -> H160 {
	Precompiles::balances_erc20()
}

/// `transfer(address,uint256)`.
pub fn encode_transfer(to: H160, amount: u128) -> Vec<u8> {
	let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
	data.extend_from_slice(&[0u8; 12]);
	data.extend_from_slice(to.as_bytes());
	data.extend_from_slice(&U256::from(amount).to_big_endian());
	data
}
