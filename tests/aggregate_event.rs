use aggregator::{config::Config, storage::Storage, Aggregator};
use multiversx_sc::{codec::TopDecode, types::*};
use multiversx_sc_scenario::{
    api::DebugApi,
    managed_address, managed_biguint, rust_biguint,
    whitebox_legacy::{BlockchainStateWrapper, TxTokenTransfer},
};

const INPUT: &[u8] = b"INPUT-123456";
const OUTPUT: &[u8] = b"OUTPUT-123456";
const SECOND: &[u8] = b"SECOND-123456";

mod pool {
    multiversx_sc::imports!();

    #[multiversx_sc::contract]
    pub trait Pool {
        #[init]
        fn init(&self) {}

        #[payable("*")]
        #[endpoint(exchange)]
        fn exchange(&self, token_out: TokenIdentifier, _min: BigUint) {
            let input = self.call_value().single();
            let caller = self.blockchain().get_caller();
            self.tx()
                .to(&caller)
                .single_esdt(&token_out, 0, &(input.amount.as_big_uint() * 2u64))
                .transfer();
            let dust = input.amount.as_big_uint() / 10u64;
            if dust > 0u64 {
                self.tx()
                    .to(&caller)
                    .payment(Payment::new(
                        input.token_identifier.clone(),
                        0,
                        dust.into_non_zero().unwrap(),
                    ))
                    .transfer();
            }
        }

        #[payable("*")]
        #[endpoint(addLiquidity)]
        fn add_liquidity(&self, _first_min: BigUint, _second_min: BigUint) {
            let caller = self.blockchain().get_caller();
            for (token, amount) in [
                (super::OUTPUT, 10_000u64),
                (super::INPUT, 100),
                (super::SECOND, 200),
            ] {
                self.tx()
                    .to(&caller)
                    .single_esdt(
                        &TokenIdentifier::from_esdt_bytes(token),
                        0,
                        &BigUint::from(amount),
                    )
                    .transfer();
            }
        }

        #[storage_mapper("first_token_id")]
        fn first_token(&self) -> SingleValueMapper<TokenIdentifier>;

        #[storage_mapper("second_token_id")]
        fn second_token(&self) -> SingleValueMapper<TokenIdentifier>;

        #[storage_mapper("first_token_reserve")]
        fn first_reserve(&self) -> SingleValueMapper<BigUint>;

        #[storage_mapper("second_token_reserve")]
        fn second_reserve(&self) -> SingleValueMapper<BigUint>;
    }
}

fn decode_payments(world: &BlockchainStateWrapper, topic: &[u8]) -> Vec<(Vec<u8>, u64)> {
    world.execute_in_managed_environment(|| {
        PaymentVec::<DebugApi>::top_decode(topic)
            .unwrap()
            .iter()
            .map(|payment| {
                assert_eq!(payment.token_nonce, 0);
                (
                    payment.token_identifier.as_managed_buffer().to_vec(),
                    payment.amount.as_big_uint().to_u64().unwrap(),
                )
            })
            .collect()
    })
}

#[test]
fn event_matches_transfers_and_transaction_fee_credits() {
    for fee_on_output in [false, true] {
        // Expected values independently specify input fees and output fees.
        for (
            referral_id,
            expected_referral_id,
            active,
            rate,
            amount,
            output,
            admin_input,
            referral_input,
            admin_output,
            referral_output,
        ) in [
            (1, 1, true, 200, 10_000, 19_400, 1_070, 200, 200, 400),
            (0, 0, true, 200, 10_000, 20_000, 1_000, 0, 0, 0),
            (99, 0, true, 200, 10_000, 20_000, 1_000, 0, 0, 0),
            (1, 0, false, 200, 10_000, 20_000, 1_000, 0, 0, 0),
            (1, 1, true, 0, 10_000, 19_800, 1_090, 0, 200, 0),
            (1, 1, true, 200, 1, 2, 0, 0, 0, 0),
        ] {
            let mut world = BlockchainStateWrapper::new();
            let user = world.create_user_account(&rust_biguint!(0));
            let contract = world.create_sc_account(
                &rust_biguint!(0),
                Some(&user),
                aggregator::contract_obj,
                "src/aggregator.rs",
            );
            let dex = world.create_sc_account(
                &rust_biguint!(0),
                Some(&user),
                pool::contract_obj,
                "tests/aggregate_event.rs",
            );
            world.set_esdt_balance(&user, INPUT, &rust_biguint!(amount));
            world.set_esdt_balance(dex.address_ref(), OUTPUT, &rust_biguint!(100_000));
            world
                .execute_tx(&user, &contract, &rust_biguint!(0), |sc| {
                    sc.init();
                    sc.set_static_fee(100);
                    assert_eq!(sc.add_referral(managed_address!(&user), rate), 1);
                    sc.set_referral_active(1, active);
                    if fee_on_output {
                        sc.add_whitelisted_token(TokenId::from(OUTPUT));
                    }
                    // Existing balances must not leak into this transaction's event.
                    sc.admin_fees()
                        .insert(TokenId::from(INPUT), managed_biguint!(7));
                    sc.admin_fees()
                        .insert(TokenId::from(OUTPUT), managed_biguint!(11));
                    sc.referrer_balances(referral_id)
                        .insert(TokenId::from(INPUT), managed_biguint!(13));
                    sc.referrer_balances(referral_id)
                        .insert(TokenId::from(OUTPUT), managed_biguint!(17));
                })
                .assert_ok();

            let result = world.execute_esdt_transfer(
                &user,
                &contract,
                INPUT,
                0,
                &rust_biguint!(amount),
                |sc| {
                    let mut tokens = MultiValueEncodedCounted::new();
                    tokens.push(TokenIdentifier::from_esdt_bytes(INPUT));
                    tokens.push(TokenIdentifier::from_esdt_bytes(OUTPUT));
                    let mut addresses = MultiValueEncodedCounted::new();
                    addresses.push(managed_address!(dex.address_ref()));
                    let mut instructions = MultiValueEncoded::new();
                    instructions.push((3u8, 1u8, 0u8, 0u8, 0u8, 0u16).into());
                    sc.aggregate(
                        managed_biguint!(output),
                        1,
                        referral_id,
                        tokens,
                        addresses,
                        MultiValueEncodedCounted::new(),
                        instructions,
                    );
                },
            );
            result.assert_ok();
            let events: Vec<_> = result
                .result_logs
                .iter()
                .filter(|log| log.topics.first().map(Vec::as_slice) == Some(b"aggregate"))
                .collect();
            assert_eq!(events.len(), 1);
            let event = events[0];
            assert_eq!(event.address, *contract.address_ref());
            assert_eq!(event.topics.len(), 7);
            assert_eq!(event.topics[1], user.as_bytes());
            assert_eq!(
                u64::top_decode(event.topics[2].as_slice()).unwrap(),
                expected_referral_id
            );
            assert_eq!(
                decode_payments(&world, &event.topics[3]),
                vec![(INPUT.to_vec(), amount)]
            );
            assert_eq!(
                decode_payments(&world, &event.topics[4]),
                vec![(OUTPUT.to_vec(), output)]
            );
            let (admin_in, admin_out, referral_in, referral_out) = if fee_on_output {
                (amount / 10, admin_output, 0, referral_output)
            } else {
                (admin_input, 0, referral_input, 0)
            };
            let expected_admin: Vec<_> = [(INPUT, admin_in), (OUTPUT, admin_out)]
                .into_iter()
                .filter(|(_, amount)| *amount > 0)
                .map(|(token, amount)| (token.to_vec(), amount))
                .collect();
            let expected_referral: Vec<_> = [(INPUT, referral_in), (OUTPUT, referral_out)]
                .into_iter()
                .filter(|(_, amount)| *amount > 0)
                .map(|(token, amount)| (token.to_vec(), amount))
                .collect();
            assert_eq!(decode_payments(&world, &event.topics[5]), expected_admin);
            assert_eq!(decode_payments(&world, &event.topics[6]), expected_referral);
            assert!(event.data.iter().all(Vec::is_empty));
            assert_eq!(result.result_logs.last().unwrap().topics[0], b"aggregate");
            world.check_esdt_balance(&user, OUTPUT, &rust_biguint!(output));
            world.check_esdt_balance(&user, INPUT, &rust_biguint!(0));
            world
                .execute_query(&contract, |sc| {
                    assert_eq!(
                        sc.admin_fees().get(&TokenId::from(INPUT)).unwrap(),
                        managed_biguint!(7 + admin_in)
                    );
                    assert_eq!(
                        sc.admin_fees().get(&TokenId::from(OUTPUT)).unwrap(),
                        managed_biguint!(11 + admin_out)
                    );
                    assert_eq!(
                        sc.referrer_balances(referral_id)
                            .get(&TokenId::from(INPUT))
                            .unwrap(),
                        managed_biguint!(13 + referral_in)
                    );
                    assert_eq!(
                        sc.referrer_balances(referral_id)
                            .get(&TokenId::from(OUTPUT))
                            .unwrap(),
                        managed_biguint!(17 + referral_out)
                    );
                })
                .assert_ok();
        }
    }
}

#[test]
fn bulk_event_and_slippage_rollback() {
    let mut world = BlockchainStateWrapper::new();
    let user = world.create_user_account(&rust_biguint!(0));
    let contract = world.create_sc_account(
        &rust_biguint!(0),
        Some(&user),
        aggregator::contract_obj,
        "src/aggregator.rs",
    );
    world.set_esdt_balance(&user, INPUT, &rust_biguint!(1_000));
    world.set_esdt_balance(&user, OUTPUT, &rust_biguint!(10_000));
    world
        .execute_tx(&user, &contract, &rust_biguint!(0), |sc| {
            sc.set_static_fee(100);
            sc.add_referral(managed_address!(&user), 200);
        })
        .assert_ok();
    let payments = [
        TxTokenTransfer {
            token_identifier: INPUT.to_vec(),
            nonce: 0,
            value: rust_biguint!(1_000),
        },
        TxTokenTransfer {
            token_identifier: OUTPUT.to_vec(),
            nonce: 0,
            value: rust_biguint!(10_000),
        },
    ];
    for min_output in [9_701, 9_700] {
        let result = world.execute_esdt_multi_transfer(&user, &contract, &payments, |sc| {
            let mut tokens = MultiValueEncodedCounted::new();
            tokens.push(TokenIdentifier::from_esdt_bytes(OUTPUT));
            sc.aggregate(
                managed_biguint!(min_output),
                0,
                1,
                tokens,
                MultiValueEncodedCounted::new(),
                MultiValueEncodedCounted::new(),
                MultiValueEncoded::new(),
            );
        });
        if min_output == 9_701 {
            result.assert_user_error("Slippage limit exceeded: have 9700, need 9701");
            assert!(result
                .result_logs
                .iter()
                .all(|log| log.topics.first().map(Vec::as_slice) != Some(b"aggregate")));
            world.check_esdt_balance(&user, INPUT, &rust_biguint!(1_000));
            world.check_esdt_balance(&user, OUTPUT, &rust_biguint!(10_000));
            world
                .execute_query(&contract, |sc| {
                    assert!(sc.admin_fees().is_empty());
                    assert!(sc.referrer_balances(1).is_empty());
                })
                .assert_ok();
        } else {
            result.assert_ok();
            let event = result
                .result_logs
                .iter()
                .find(|log| log.topics.first().map(Vec::as_slice) == Some(b"aggregate"))
                .unwrap();
            assert_eq!(
                decode_payments(&world, &event.topics[3]),
                vec![(INPUT.to_vec(), 1_000), (OUTPUT.to_vec(), 10_000)]
            );
            assert_eq!(
                decode_payments(&world, &event.topics[4]),
                vec![(OUTPUT.to_vec(), 9_700)]
            );
            assert_eq!(
                decode_payments(&world, &event.topics[5]),
                vec![(INPUT.to_vec(), 1_000), (OUTPUT.to_vec(), 100)]
            );
            assert_eq!(
                decode_payments(&world, &event.topics[6]),
                vec![(OUTPUT.to_vec(), 200)]
            );
            world.check_esdt_balance(&user, OUTPUT, &rust_biguint!(9_700));
        }
    }
}

#[test]
fn event_includes_lp_creation_dust() {
    use pool::Pool;
    let mut world = BlockchainStateWrapper::new();
    let user = world.create_user_account(&rust_biguint!(0));
    let contract = world.create_sc_account(
        &rust_biguint!(0),
        Some(&user),
        aggregator::contract_obj,
        "src/aggregator.rs",
    );
    let dex = world.create_sc_account(
        &rust_biguint!(0),
        Some(&user),
        pool::contract_obj,
        "tests/aggregate_event.rs",
    );
    world.set_esdt_balance(&user, INPUT, &rust_biguint!(10_000));
    world.set_esdt_balance(&user, SECOND, &rust_biguint!(20_000));
    world.set_esdt_balance(dex.address_ref(), OUTPUT, &rust_biguint!(10_000));
    world
        .execute_tx(&user, &dex, &rust_biguint!(0), |sc| {
            sc.first_token()
                .set(TokenIdentifier::from_esdt_bytes(INPUT));
            sc.second_token()
                .set(TokenIdentifier::from_esdt_bytes(SECOND));
            sc.first_reserve().set(managed_biguint!(1_000_000));
            sc.second_reserve().set(managed_biguint!(2_000_000));
        })
        .assert_ok();
    world
        .execute_tx(&user, &contract, &rust_biguint!(0), |sc| {
            sc.set_static_fee(100);
            sc.add_referral(managed_address!(&user), 200);
        })
        .assert_ok();
    let payments = [
        TxTokenTransfer {
            token_identifier: INPUT.to_vec(),
            nonce: 0,
            value: rust_biguint!(10_000),
        },
        TxTokenTransfer {
            token_identifier: SECOND.to_vec(),
            nonce: 0,
            value: rust_biguint!(20_000),
        },
    ];
    let result = world.execute_esdt_multi_transfer(&user, &contract, &payments, |sc| {
        let mut tokens = MultiValueEncodedCounted::new();
        for token in [INPUT, SECOND, OUTPUT] {
            tokens.push(TokenIdentifier::from_esdt_bytes(token));
        }
        let mut addresses = MultiValueEncodedCounted::new();
        addresses.push(managed_address!(dex.address_ref()));
        let mut instructions = MultiValueEncoded::new();
        instructions.push((13u8, 0u8, 0u8, 1u8, 0u8, 0u16).into());
        sc.aggregate(
            managed_biguint!(9_700),
            2,
            1,
            tokens,
            addresses,
            MultiValueEncodedCounted::new(),
            instructions,
        );
    });
    result.assert_ok();
    let event = result
        .result_logs
        .iter()
        .find(|log| log.topics.first().map(Vec::as_slice) == Some(b"aggregate"))
        .unwrap();
    assert_eq!(
        decode_payments(&world, &event.topics[3]),
        vec![(INPUT.to_vec(), 10_000), (SECOND.to_vec(), 20_000)]
    );
    assert_eq!(
        decode_payments(&world, &event.topics[4]),
        vec![(OUTPUT.to_vec(), 9_700)]
    );
    assert_eq!(
        decode_payments(&world, &event.topics[5]),
        vec![
            (INPUT.to_vec(), 100),
            (SECOND.to_vec(), 200),
            (OUTPUT.to_vec(), 100)
        ]
    );
    assert_eq!(
        decode_payments(&world, &event.topics[6]),
        vec![(OUTPUT.to_vec(), 200)]
    );
    world.check_esdt_balance(&user, OUTPUT, &rust_biguint!(9_700));
    world
        .execute_query(&contract, |sc| {
            for (token, amount) in [(INPUT, 100), (SECOND, 200), (OUTPUT, 100)] {
                assert_eq!(
                    sc.admin_fees().get(&TokenId::from(token)).unwrap(),
                    managed_biguint!(amount)
                );
            }
        })
        .assert_ok();
}

#[test]
fn native_input_output_with_no_referral() {
    let mut world = BlockchainStateWrapper::new();
    let user = world.create_user_account(&rust_biguint!(10_000));
    let contract = world.create_sc_account(
        &rust_biguint!(0),
        Some(&user),
        aggregator::contract_obj,
        "src/aggregator.rs",
    );
    let result = world.execute_tx(&user, &contract, &rust_biguint!(10_000), |sc| {
        sc.aggregate(
            managed_biguint!(10_000),
            aggregator::types::IDX_EGLD,
            0,
            MultiValueEncodedCounted::new(),
            MultiValueEncodedCounted::new(),
            MultiValueEncodedCounted::new(),
            MultiValueEncoded::new(),
        );
    });
    result.assert_ok();
    let event = result
        .result_logs
        .iter()
        .find(|log| log.topics.first().map(Vec::as_slice) == Some(b"aggregate"))
        .unwrap();
    let expected = vec![(b"EGLD-000000".to_vec(), 10_000)];
    assert_eq!(decode_payments(&world, &event.topics[3]), expected);
    assert_eq!(decode_payments(&world, &event.topics[4]), expected);
    assert!(decode_payments(&world, &event.topics[5]).is_empty());
    assert!(decode_payments(&world, &event.topics[6]).is_empty());
    world.check_egld_balance(&user, &rust_biguint!(10_000));
}
