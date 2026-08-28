// Call context gate on the balances-erc20 precompile at `0x802`, driven through
// a real `pallet_evm::runner::stack::Runner`.
//
// The precompile reads the funds owner from `context().caller`. DELEGATECALL
// rebinds that field to the outer caller, so a dispatcher that only matches on
// the code address lets any contract move a third party's native balance with
// no approval. The set is built with `PrecompileSetBuilder`, whose checks reject
// borrowed code while leaving plain CALL open to contracts.

mod common;

use common::new_test_ext;
use frame_support::traits::tokens::fungible::Mutate;
use numen_runtime::{AccountId, Balances, Runtime, UNIT};
use pallet_evm::{AddressMapping, Runner};
use sp_core::{H160, U256};

const MAX_FEE_PER_GAS: u64 = 2_000_000_000_000;
const GAS_LIMIT: u64 = 1_000_000;
const ERC20_PRECOMPILE: u64 = 0x0802;

fn evm_account(addr: H160) -> AccountId {
    <Runtime as pallet_evm::Config>::AddressMapping::into_account_id(addr)
}

// Runtime bytecode that forwards its calldata verbatim into `0x802` via
// DELEGATECALL, so the precompile sees the *outer* caller as `msg.sender`, then
// returns the inner output so the test can read the revert reason.
//
//   CALLDATACOPY(0, 0, cds)                    copy calldata into memory
//   DELEGATECALL(gas, 0x0802, 0, cds, 0, 0)    forward it
//   POP                                        drop the success flag
//   RETURNDATACOPY(0, 0, rds)                  keep whatever came back
//   RETURN(0, rds)                             hand it to the caller
//
// A 12 byte constructor returns the 29 byte runtime code that follows it.
const DELEGATECALL_FORWARDER_INIT: [u8; 41] = [
    // constructor CODECOPYs the 29 byte runtime to memory 0 then RETURNs it
    0x60, 0x1d, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x1d, 0x60, 0x00, 0xf3,
    // forwarder body, CALLDATACOPY(dest=0, off=0, len=CALLDATASIZE)
    0x36, 0x60, 0x00, 0x60, 0x00, 0x37,
    // DELEGATECALL(gas=GAS, addr=0x0802, argsOff=0, argsLen=CALLDATASIZE, retOff=0, retLen=0)
    0x60, 0x00, 0x60, 0x00, 0x36, 0x60, 0x00, 0x61, 0x08, 0x02, 0x5a, 0xf4,
    // POP the success flag
    0x50,
    // RETURNDATACOPY(dest=0, off=0, len=RETURNDATASIZE)
    0x3d, 0x60, 0x00, 0x60, 0x00, 0x3e,
    // RETURN(off=0, len=RETURNDATASIZE)
    0x3d, 0x60, 0x00, 0xf3,
];

// Same forwarder with CALL in place of DELEGATECALL. The precompile then sees
// the contract itself as `msg.sender`, which is how a contract spends its own
// balance.
//
//   CALLDATACOPY(0, 0, cds)                       copy calldata into memory
//   CALL(gas, 0x0802, 0, 0, cds, 0, 0)            forward it
//   POP                                           drop the success flag
//   RETURNDATACOPY(0, 0, rds)                     keep whatever came back
//   RETURN(0, rds)                                hand it to the caller
const CALL_FORWARDER_INIT: [u8; 43] = [
    // constructor CODECOPYs the 31 byte runtime to memory 0 then RETURNs it
    0x60, 0x1f, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x1f, 0x60, 0x00, 0xf3,
    // forwarder body, CALLDATACOPY(dest=0, off=0, len=CALLDATASIZE)
    0x36, 0x60, 0x00, 0x60, 0x00, 0x37,
    // CALL(gas=GAS, addr=0x0802, value=0, argsOff=0, argsLen=CALLDATASIZE, retOff=0, retLen=0)
    0x60, 0x00, 0x60, 0x00, 0x36, 0x60, 0x00, 0x60, 0x00, 0x61, 0x08, 0x02, 0x5a, 0xf1,
    // POP the success flag
    0x50,
    // RETURNDATACOPY(dest=0, off=0, len=RETURNDATASIZE)
    0x3d, 0x60, 0x00, 0x60, 0x00, 0x3e,
    // RETURN(off=0, len=RETURNDATASIZE)
    0x3d, 0x60, 0x00, 0xf3,
];

/// `transfer(address,uint256)`.
fn encode_transfer(to: H160, amount: u128) -> Vec<u8> {
    let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(to.as_bytes());
    data.extend_from_slice(&U256::from(amount).to_big_endian());
    data
}

fn deploy_forwarder(deployer: H160, init_code: &[u8]) -> H160 {
    let info = <Runtime as pallet_evm::Config>::Runner::create(
        deployer,
        init_code.to_vec(),
        U256::zero(),
        GAS_LIMIT,
        Some(U256::from(MAX_FEE_PER_GAS)),
        None,
        None,
        Vec::new(),
        Vec::new(),
        true,
        true,
        None,
        None,
        <Runtime as pallet_evm::Config>::config(),
    )
    .map_err(|e| e.error)
    .expect("create dispatches without runtime error");
    assert!(info.exit_reason.is_succeed(), "deploy failed: {:?}", info.exit_reason);
    info.value
}

fn call_contract(caller: H160, target: H160, data: Vec<u8>) -> fp_evm::CallInfo {
    <Runtime as pallet_evm::Config>::Runner::call(
        caller,
        target,
        data,
        U256::zero(),
        GAS_LIMIT,
        Some(U256::from(MAX_FEE_PER_GAS)),
        None,
        None,
        Vec::new(),
        Vec::new(),
        true,
        true,
        None,
        None,
        None,
        <Runtime as pallet_evm::Config>::config(),
    )
    .map_err(|e| e.error)
    .expect("call dispatches without runtime error")
}

/// A victim who calls an attacker's ordinary-looking contract keeps their native
/// balance. The contract DELEGATECALLs the ERC-20 precompile, which would read
/// the victim as `msg.sender`, so the precompile refuses to run at all.
#[test]
fn delegatecall_into_erc20_precompile_is_rejected() {
    new_test_ext().execute_with(|| {
        let deployer = H160::from_low_u64_be(0xDEAD_BEEF);
        let victim = H160::from_low_u64_be(0x0000_1C71); // "victim"
        let attacker = H160::from_low_u64_be(0x0000_0BAD);

        // Deployer pays gas; victim holds the loot; attacker starts empty.
        Balances::set_balance(&evm_account(deployer), 100 * UNIT);
        let victim_start = 7_500 * UNIT;
        Balances::set_balance(&evm_account(victim), victim_start);
        assert_eq!(Balances::free_balance(evm_account(attacker)), 0);

        let forwarder = deploy_forwarder(deployer, &DELEGATECALL_FORWARDER_INIT);

        // Victim calls the forwarder. No `approve` was ever issued. The steal
        // amount is baked into the calldata the victim unwittingly forwards.
        let steal = 1_000 * UNIT;
        let info = call_contract(victim, forwarder, encode_transfer(attacker, steal));
        assert!(
            info.exit_reason.is_succeed(),
            "the forwarder itself must not fail: {:?}",
            info.exit_reason
        );

        // The forwarder bubbles up whatever the precompile returned.
        let returned = String::from_utf8_lossy(&info.value).into_owned();
        assert!(
            returned.contains("Cannot be called with DELEGATECALL or CALLCODE"),
            "expected the delegate call gate to fire, got {returned:?}",
        );

        assert_eq!(
            Balances::free_balance(evm_account(attacker)),
            0,
            "attacker must not receive anything",
        );
        let victim_end = Balances::free_balance(evm_account(victim));
        assert!(
            victim_end > victim_start - steal,
            "victim must only pay gas, never the steal amount: {victim_start} -> {victim_end}",
        );
    });
}

/// Sanity contrast. A direct (non-delegate) call to the precompile debits the
/// caller's own account, as intended.
#[test]
fn direct_call_to_erc20_precompile_debits_the_real_caller() {
    new_test_ext().execute_with(|| {
        let caller = H160::from_low_u64_be(0x0000_C1A1);
        let recipient = H160::from_low_u64_be(0x0000_0BAD);
        Balances::set_balance(&evm_account(caller), 7_500 * UNIT);
        assert_eq!(Balances::free_balance(evm_account(recipient)), 0);

        let amount = 1_000 * UNIT;
        let precompile = H160::from_low_u64_be(ERC20_PRECOMPILE);
        let info = call_contract(caller, precompile, encode_transfer(recipient, amount));
        assert!(info.exit_reason.is_succeed(), "{:?}", info.exit_reason);

        // Funds come out of the caller, never a third party.
        assert_eq!(Balances::free_balance(evm_account(recipient)), amount);
    });
}

/// Contracts must keep reaching the precompile through a plain CALL and spend
/// their own balance. This is the half of the call context policy that a missing
/// `CallableByContract` would silently break, and no other test covers it
/// because every other caller here is an EOA.
#[test]
fn contract_call_to_erc20_precompile_debits_the_calling_contract() {
    new_test_ext().execute_with(|| {
        let deployer = H160::from_low_u64_be(0xDEAD_BEEF);
        let recipient = H160::from_low_u64_be(0x0000_B0B0);
        Balances::set_balance(&evm_account(deployer), 100 * UNIT);

        let forwarder = deploy_forwarder(deployer, &CALL_FORWARDER_INIT);
        let forwarder_start = 5_000 * UNIT;
        Balances::set_balance(&evm_account(forwarder), forwarder_start);
        assert_eq!(Balances::free_balance(evm_account(recipient)), 0);

        let amount = 1_000 * UNIT;
        let info = call_contract(deployer, forwarder, encode_transfer(recipient, amount));
        assert!(
            info.exit_reason.is_succeed(),
            "contract call reverted: {:?}",
            info.exit_reason
        );
        // Length first, so a rejected contract call reports the revert reason
        // instead of panicking inside `from_big_endian`.
        assert_eq!(
            info.value.len(),
            32,
            "expected a single ABI word, got {:?}",
            String::from_utf8_lossy(&info.value),
        );
        assert_eq!(U256::from_big_endian(&info.value), U256::one(), "transfer returns bool true");

        assert_eq!(Balances::free_balance(evm_account(recipient)), amount);
        assert_eq!(
            Balances::free_balance(evm_account(forwarder)),
            forwarder_start - amount,
            "the contract is the spender, gas stays on the deployer",
        );
    });
}
