//! Fees held for claimants must never be spent by an aggregation, whatever pool
//! addresses and input tokens the caller supplies.

use aggregator::{constants::WRAPPER_SC, Aggregator};
use multiversx_sc::types::*;
use multiversx_sc_scenario::{
    api::DebugApi,
    managed_address, managed_biguint, rust_biguint,
    whitebox_legacy::{BlockchainStateWrapper, ContractObjWrapper, TxTokenTransfer},
};

const MEX: &[u8] = b"MEX-455c57";
const USDC: &[u8] = b"USDC-c76f1f";
const WEGLD: &[u8] = b"WEGLD-bd4d79";
const IDX_EGLD: u8 = 254;
const NONE: u8 = 255;

type Sc = aggregator::ContractObj<DebugApi>;
type Contract = ContractObjWrapper<Sc, fn() -> Sc>;

mod wrapper {
    multiversx_sc::imports!();

    #[multiversx_sc::contract]
    pub trait Wrapper {
        #[init]
        fn init(&self) {}

        #[payable("EGLD")]
        #[endpoint(wrapEgld)]
        fn wrap_egld(&self) {
            let amount = self.call_value().egld().clone_value();
            self.tx()
                .to(&self.blockchain().get_caller())
                .single_esdt(&TokenIdentifier::from_esdt_bytes(super::WEGLD), 0, &amount)
                .transfer();
        }
    }
}

/// Untrusted Jex-shaped pool: reports arbitrary reserves, answers swaps with
/// whatever token it is configured to return, and keeps everything sent to addLiquidity.
mod untrusted_pool {
    multiversx_sc::imports!();

    #[multiversx_sc::contract]
    pub trait UntrustedPool {
        #[init]
        fn init(&self) {}

        #[payable("*")]
        #[endpoint(swapTokensFixedInput)]
        fn swap(&self, _min: BigUint) {
            self.tx()
                .to(&self.blockchain().get_caller())
                .single_esdt(&self.return_token().get(), 0, &self.return_amount().get())
                .transfer();
        }

        #[payable("*")]
        #[endpoint(addLiquidity)]
        fn add_liquidity(&self, _first_min: BigUint, _second_min: BigUint) {
            self.tx()
                .to(&self.blockchain().get_caller())
                .single_esdt(&self.second_token().get(), 0, &BigUint::from(1u64))
                .transfer();
        }

        #[storage_mapper("first_token_id")]
        fn first_token(&self) -> SingleValueMapper<TokenIdentifier>;

        #[storage_mapper("second_token_id")]
        fn second_token(&self) -> SingleValueMapper<TokenIdentifier>;

        #[storage_mapper("first_token_reserve")]
        fn first_reserve(&self) -> SingleValueMapper<BigUint>;

        #[storage_mapper("second_token_reserve")]
        fn second_reserve(&self) -> SingleValueMapper<BigUint>;

        #[storage_mapper("return_token")]
        fn return_token(&self) -> SingleValueMapper<TokenIdentifier>;

        #[storage_mapper("return_amount")]
        fn return_amount(&self) -> SingleValueMapper<BigUint>;
    }
}

/// World with an untrusted caller and an aggregator already holding `held_egld` of claimable fees.
fn setup(held_egld: u64) -> (BlockchainStateWrapper, Address, Contract) {
    let mut world = BlockchainStateWrapper::new();
    let owner = world.create_user_account(&rust_biguint!(0));
    let caller = world.create_user_account(&rust_biguint!(0));
    let contract = world.create_sc_account(
        &rust_biguint!(held_egld),
        Some(&owner),
        aggregator::contract_obj as fn() -> Sc,
        "src/aggregator.rs",
    );
    (world, caller, contract)
}

/// Call `xo` with one instruction, no fees and no referral.
fn xo(
    sc: Sc,
    min_out: u64,
    token_out: u8,
    tokens: &[&[u8]],
    pool: Option<&Address>,
    instruction: (u8, u8, u8, u8, u8, u16),
) {
    let mut registry = MultiValueEncodedCounted::new();
    for token in tokens {
        registry.push(TokenIdentifier::from_esdt_bytes(*token));
    }
    let mut addresses = MultiValueEncodedCounted::new();
    if let Some(pool) = pool {
        addresses.push(managed_address!(pool));
    }
    let mut instructions = MultiValueEncoded::new();
    instructions.push(instruction.into());
    sc.aggregate(
        managed_biguint!(min_out),
        token_out,
        0,
        registry,
        addresses,
        MultiValueEncodedCounted::new(),
        instructions,
    );
}

/// An ESDT paid into an EGLD-only action must not be swapped for the contract's own
/// held EGLD. Same input rule for every EGLD-only action:
/// Wrapping (18), XoxnoLiquidStaking (20), HatomLiquidStaking (22).
#[test]
fn egld_only_actions_reject_esdt_input() {
    for action in [18u8, 20, 22] {
        let (mut world, caller, contract) = setup(100);
        world.set_esdt_balance(&caller, MEX, &rust_biguint!(50));
        world
            .execute_esdt_transfer(&caller, &contract, MEX, 0, &rust_biguint!(50), |sc| {
                xo(sc, 1, 0, &[MEX], None, (action, 0, 0, NONE, 0, 255));
            })
            .assert_user_error("Input token must be EGLD");
        world.check_egld_balance(contract.address_ref(), &rust_biguint!(100));
    }
}

/// Paying real EGLD still wraps, and the held EGLD is untouched.
#[test]
fn wrapping_egld_keeps_held_egld() {
    let (mut world, caller, contract) = setup(100);
    let wrapper = Address::from(WRAPPER_SC);
    world.create_sc_account_fixed_address(
        &wrapper,
        &rust_biguint!(0),
        None,
        wrapper::contract_obj,
        "tests/held_balance_guard.rs#wrapper",
    );
    world.set_esdt_balance(&wrapper, WEGLD, &rust_biguint!(1_000));
    world.set_egld_balance(&caller, &rust_biguint!(50));
    world
        .execute_tx(&caller, &contract, &rust_biguint!(50), |sc| {
            xo(sc, 50, 0, &[WEGLD], None, (18, IDX_EGLD, 0, NONE, 0, 255));
        })
        .assert_ok();
    world.check_esdt_balance(&caller, WEGLD, &rust_biguint!(50));
    world.check_egld_balance(contract.address_ref(), &rust_biguint!(100));
}

/// JexAddLiquidity zap through a pool that answers the pre-swap with the wrong token.
/// Its amount must not be counted as USDC and paid out of the held USDC.
#[test]
fn zap_rejects_swap_returning_wrong_token() {
    let (mut world, caller, contract) = setup(0);
    let pool = world.create_sc_account(
        &rust_biguint!(0),
        Some(&caller),
        untrusted_pool::contract_obj,
        "tests/held_balance_guard.rs#pool",
    );
    // Held fees owed to claimants.
    world.set_esdt_balance(contract.address_ref(), USDC, &rust_biguint!(1_000_000));
    world.set_esdt_balance(pool.address_ref(), MEX, &rust_biguint!(1_000_000));
    world.set_esdt_balance(pool.address_ref(), USDC, &rust_biguint!(10));
    world.set_esdt_balance(&caller, MEX, &rust_biguint!(1_000));
    world.set_esdt_balance(&caller, USDC, &rust_biguint!(1));
    world
        .execute_tx(&caller, &pool, &rust_biguint!(0), |sc| {
            use untrusted_pool::UntrustedPool;
            sc.first_token().set(TokenIdentifier::from_esdt_bytes(MEX));
            sc.second_token()
                .set(TokenIdentifier::from_esdt_bytes(USDC));
            sc.first_reserve().set(managed_biguint!(1_000));
            sc.second_reserve().set(managed_biguint!(1_000));
            sc.return_token().set(TokenIdentifier::from_esdt_bytes(MEX));
            sc.return_amount().set(managed_biguint!(900_000));
        })
        .assert_ok();

    let payments = [(MEX, 1_000u64), (USDC, 1)].map(|(token, value)| TxTokenTransfer {
        token_identifier: token.to_vec(),
        nonce: 0,
        value: rust_biguint!(value),
    });
    world
        .execute_esdt_multi_transfer(&caller, &contract, &payments, |sc| {
            let pool = Some(pool.address_ref());
            xo(sc, 1, 1, &[MEX, USDC], pool, (13, 0, 0, 1, 0, 0));
        })
        .assert_user_error("Zap swap returned unexpected token");
    world.check_esdt_balance(contract.address_ref(), USDC, &rust_biguint!(1_000_000));
    world.check_esdt_balance(pool.address_ref(), USDC, &rust_biguint!(10));
}
