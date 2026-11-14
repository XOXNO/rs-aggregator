use aggregator::{config::Config, storage::Storage, Aggregator};
use multiversx_sc::types::*;
use multiversx_sc_scenario::{
    managed_address, managed_biguint, rust_biguint, whitebox_legacy::BlockchainStateWrapper,
};

const OUTPUT: &[u8] = b"OUTPUT-123456";
const SECOND: &[u8] = b"SECOND-123456";

/// Contract holds 300 OUTPUT and 50 SECOND. Referral 1 (owned by `referrer`)
/// has 100 OUTPUT accrued and nothing in SECOND.
#[test]
fn claim_referral_fees_pays_each_accrued_amount_once() {
    // (requested token list, expected OUTPUT paid to referrer, expected OUTPUT left in contract)
    for (requested, expected_paid, expected_left) in [
        // Duplicates must not multiply the payout.
        (vec![OUTPUT, OUTPUT, OUTPUT], 100, 200),
        // Unaccrued token is a no-op; accrued token is paid once.
        (vec![SECOND, OUTPUT, SECOND], 100, 200),
        // Empty list claims every accrued token.
        (vec![], 100, 200),
    ] {
        let mut world = BlockchainStateWrapper::new();
        let owner = world.create_user_account(&rust_biguint!(0));
        let referrer = world.create_user_account(&rust_biguint!(0));
        let anyone = world.create_user_account(&rust_biguint!(0));
        let contract = world.create_sc_account(
            &rust_biguint!(0),
            Some(&owner),
            aggregator::contract_obj,
            "src/aggregator.rs",
        );
        world.set_esdt_balance(contract.address_ref(), OUTPUT, &rust_biguint!(300));
        world.set_esdt_balance(contract.address_ref(), SECOND, &rust_biguint!(50));
        world
            .execute_tx(&owner, &contract, &rust_biguint!(0), |sc| {
                sc.init();
                assert_eq!(sc.add_referral(managed_address!(&referrer), 200), 1);
                sc.referrer_balances(1)
                    .insert(TokenId::from(OUTPUT), managed_biguint!(100));
            })
            .assert_ok();

        world
            .execute_tx(&anyone, &contract, &rust_biguint!(0), |sc| {
                let mut tokens = MultiValueEncoded::new();
                for token in &requested {
                    tokens.push(TokenId::from(*token));
                }
                sc.claim_referral_fees(1, tokens);
            })
            .assert_ok();

        world.check_esdt_balance(&referrer, OUTPUT, &rust_biguint!(expected_paid));
        world.check_esdt_balance(
            contract.address_ref(),
            OUTPUT,
            &rust_biguint!(expected_left),
        );
        // Accrued balance is fully consumed, so a second claim pays nothing.
        world
            .execute_tx(&anyone, &contract, &rust_biguint!(0), |sc| {
                let mut tokens = MultiValueEncoded::new();
                tokens.push(TokenId::from(OUTPUT));
                sc.claim_referral_fees(1, tokens);
            })
            .assert_ok();
        world.check_esdt_balance(&referrer, OUTPUT, &rust_biguint!(expected_paid));
    }
}

/// When the contract records more than it holds, claims pay what is
/// held, clear the recorded balance, and do not block fully funded tokens.
#[test]
fn underfunded_claims_pay_held_amount_and_reset() {
    let mut world = BlockchainStateWrapper::new();
    let owner = world.create_user_account(&rust_biguint!(0));
    let referrer = world.create_user_account(&rust_biguint!(0));
    let contract = world.create_sc_account(
        &rust_biguint!(0),
        Some(&owner),
        aggregator::contract_obj,
        "src/aggregator.rs",
    );
    // OUTPUT: 40 held vs 100 (referral) + 30 (admin) recorded. SECOND: fully funded.
    world.set_esdt_balance(contract.address_ref(), OUTPUT, &rust_biguint!(40));
    world.set_esdt_balance(contract.address_ref(), SECOND, &rust_biguint!(50));
    world
        .execute_tx(&owner, &contract, &rust_biguint!(0), |sc| {
            sc.init();
            sc.add_referral(managed_address!(&referrer), 200);
            for (token, amount) in [(OUTPUT, 100u64), (SECOND, 20)] {
                sc.referrer_balances(1)
                    .insert(TokenId::from(token), managed_biguint!(amount));
            }
            for (token, amount) in [(OUTPUT, 30u64), (SECOND, 30)] {
                sc.admin_fees()
                    .insert(TokenId::from(token), managed_biguint!(amount));
            }
        })
        .assert_ok();

    world
        .execute_tx(&owner, &contract, &rust_biguint!(0), |sc| {
            sc.claim_referral_fees(1, MultiValueEncoded::new());
        })
        .assert_ok();
    world.check_esdt_balance(&referrer, OUTPUT, &rust_biguint!(40));
    world.check_esdt_balance(&referrer, SECOND, &rust_biguint!(20));

    // OUTPUT is now empty: the admin claim skips it and still pays SECOND.
    world
        .execute_tx(&owner, &contract, &rust_biguint!(0), |sc| {
            sc.claim_admin_fees(managed_address!(&owner));
        })
        .assert_ok();
    world.check_esdt_balance(&owner, OUTPUT, &rust_biguint!(0));
    world.check_esdt_balance(&owner, SECOND, &rust_biguint!(30));
    world
        .execute_query(&contract, |sc| {
            assert!(sc.referrer_balances(1).is_empty());
            assert!(sc.admin_fees().is_empty());
        })
        .assert_ok();
}
