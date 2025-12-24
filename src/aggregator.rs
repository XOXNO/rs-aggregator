#![no_std]

multiversx_sc::imports!();
multiversx_sc::derive_imports!();

pub mod constants;
pub mod proxies;
pub mod storage;
pub mod types;
pub mod vault;
pub mod zap;

use constants::{
    HATOM_STAKING, LXOXNO_STAKING, MIN_INTERNAL_OUTPUT, ONE_DEX_ROUTER, WRAPPER_SC, XEGLD_STAKING,
};
use multiversx_sc::chain_core::EGLD_000000_TOKEN_IDENTIFIER;
use types::{AmountMode, Instruction};
use vault::Vault;

/// MultiversX DEX Aggregator with LP Support
///
/// Executes swap paths from the arb-algo aggregator, supporting:
/// - Token to Token swaps with splits and hops
/// - Token to LP minting
/// - LP to Token burning
/// - LP to LP conversion
#[multiversx_sc::contract]
pub trait Aggregator: storage::Storage {
    #[init]
    fn init(&self) {}

    #[upgrade]
    fn upgrade(&self) {}

    // --- Main Aggregation Endpoint ---

    /// Execute a sequence of aggregator instructions
    ///
    /// # Arguments
    /// * `instructions` - Ordered list of operations to execute
    /// * `min_amount_out` - Minimum expected output amount (slippage protection)
    /// * `token_out` - Expected output token identifier
    ///
    /// # Returns
    /// All remaining vault tokens are sent back to caller
    #[payable("*")]
    #[endpoint(xo)]
    fn aggregate(
        &self,
        min_amount_out: BigUint<Self::Api>,
        token_out: TokenId<Self::Api>,
        instructions: MultiValueEncoded<Instruction<Self::Api>>,
    ) {
        // 1. Initialize vault from incoming payments
        let payment = self.call_value().single();

        let mut vault = Vault::from_payment(payment);

        // 2. Execute each instruction sequentially
        for instruction in instructions {
            self.execute_instruction(&mut vault, &instruction);
        }

        // 3. Verify minimum output amount
        let current_balance = vault.balance_of(&token_out);

        require!(
            vault.has_minimum(&token_out, &min_amount_out),
            "Output amount below minimum, expected at least {} of {}",
            min_amount_out,
            current_balance
        );

        // 4. Return all vault contents to caller
        let caller = self.blockchain().get_caller();
        let output_payments = vault.get_all_payments();

        self.tx()
            .to(caller)
            .payment(output_payments)
            .transfer_if_not_empty();
    }

    #[proxy]
    fn proxy_call(&self, address: ManagedAddress) -> proxies::Proxy<Self::Api>;

    // --- Instruction Execution ---

    /// Execute a single instruction by dispatching to the appropriate DEX proxy
    fn execute_instruction(&self, vault: &mut Vault<Self::Api>, instr: &Instruction<Self::Api>) {
        let mut input_payments = ManagedVec::new();

        if let Some(inputs) = &instr.inputs {
            // 1. Withdraw all required inputs from vault
            for input in inputs.iter() {
                // Normalize token to handle "EGLD" -> "EGLD-000000"
                let token = if input.token.is_empty() {
                    TokenId::from(EGLD_000000_TOKEN_IDENTIFIER.as_bytes())
                } else {
                    TokenId::from(input.token.clone())
                };

                let actual_amount = match &input.mode {
                    AmountMode::Fixed(amount) => vault.withdraw(&token, amount),
                    AmountMode::Ppm(ppm) => vault.withdraw_ppm(&token, ppm),
                    AmountMode::All => vault.withdraw_all(&token),
                    AmountMode::PrevAmount => {
                        let prev_result = vault.get_prev_result();
                        require!(prev_result.is_some(), "PrevAmount not available");
                        let prev_value = prev_result.as_ref().unwrap();
                        require!(
                            token == prev_value.token_identifier,
                            "PrevAmount token mismatch"
                        );
                        vault.withdraw(&token, prev_value.amount.clone().as_big_uint())
                    }
                };

                require!(actual_amount > 0u64, "Zero input amount");

                input_payments.push(Payment::new(
                    token,
                    0u64,
                    actual_amount.into_non_zero().unwrap(),
                ));
            }
        } else {
            let prev = vault.get_prev_result().clone().unwrap();
            // Withdraw from vault to keep it in sync with actual contract holdings
            vault.withdraw(&prev.token_identifier, prev.amount.as_big_uint());
            input_payments.push(prev);
        }

        // 2. Dispatch to appropriate proxy
        self.dispatch_to_proxy(vault, instr, &input_payments);
    }

    // --- Dispatch Logic ---

    /// Dispatch instruction to the appropriate DEX proxy
    fn dispatch_to_proxy(
        &self,
        vault: &mut Vault<Self::Api>,
        instr: &Instruction<Self::Api>,
        payments: &ManagedVec<Payment<Self::Api>>,
    ) {
        // For zappable add_liquidity actions, use pre-balance optimization
        if self.is_zappable_add_liquidity(&instr.action) {
            return self.pre_balance_and_add_liquidity(vault, instr, payments);
        }

        let min = BigUint::from(MIN_INTERNAL_OUTPUT);

        let mut call = self.get_proxy_call(instr, payments);

        // Execute the appropriate proxy call based on DEX type
        let back_transfers = match &instr.action {
            // --- xExchange ---
            types::ActionType::XExchangeSwap(token_out) => call
                .xexchange(token_out, min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::XExchangeAddLiquidity => call
                .xdex_add_liquidity(min.clone(), min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::XExchangeRemoveLiquidity => call
                .xdex_remove_liquidity(min.clone(), min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- AshSwap V1 (Stable) ---
            types::ActionType::AshSwapPoolSwap(token_out) => call
                .ash_exchange_stable(token_out, min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::AshSwapPoolAddLiquidity => call
                .ash_add_liquidity_stable(min, self.blockchain().get_sc_address())
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::AshSwapPoolRemoveLiquidity(out_tokens) => call
                .ash_remove_liquidity_stable({
                    let mut mv = MultiValueEncoded::new();
                    for _ in 0..*out_tokens {
                        mv.push(min.clone());
                    }
                    mv
                })
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- AshSwap V2 (Crypto) ---
            types::ActionType::AshSwapV2Swap => call
                .ash_exchange_crypto(min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::AshSwapV2AddLiquidity => call
                .ash_add_liquidity_crypto(
                    min,
                    OptionalValue::<multiversx_sc::types::ManagedAddress<Self::Api>>::None,
                )
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::AshSwapV2RemoveLiquidity(out_tokens) => call
                .ash_remove_liquidity_crypto(
                    {
                        let mut mv = ManagedVec::new();
                        for _ in 0..*out_tokens {
                            mv.push(min.clone());
                        }
                        mv
                    },
                    OptionalValue::<multiversx_sc::types::ManagedAddress<Self::Api>>::None,
                )
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- OneDex ---
            types::ActionType::OneDexSwap(token_out) => {
                let mut path = MultiValueEncoded::new();
                for input in payments.iter() {
                    unsafe {
                        path.push(input.token_identifier.clone().into_esdt_unchecked());
                    }
                }
                path.push(token_out.clone());
                call.onedex(min, false, path)
                    .payment(payments)
                    .returns(ReturnsBackTransfersReset)
                    .sync_call()
            }
            types::ActionType::OneDexAddLiquidity(_) => call
                .xdex_add_liquidity(min.clone(), min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::OneDexRemoveLiquidity => call
                .onedex_remove_liquidity(min.clone(), min.clone(), false)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- Jex (CPMM) ---
            types::ActionType::JexSwap => call
                .jex(min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::JexAddLiquidity => call
                .xdex_add_liquidity(min.clone(), min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::JexRemoveLiquidity => call
                .xdex_remove_liquidity(min.clone(), min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- Jex (Stable) ---
            types::ActionType::JexStableSwap(token_out) => call
                .jex_swap_stable(token_out, min * 2u64)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::JexStableAddLiquidity => call
                .jex_add_liquidity_stable(min * 2u64)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::JexStableRemoveLiquidity => call
                .jex(min * 2u64)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- EGLD Wrapping ---
            types::ActionType::Wrapping => call
                .wrap_egld()
                .egld(payments.get(0).amount.as_big_uint())
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::UnWrapping => call
                .unwrap_egld()
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- Liquid Staking ---
            types::ActionType::XoxnoLiquidStaking | types::ActionType::LXoxnoLiquidStaking => call
                .delegate(OptionalValue::<multiversx_sc::types::ManagedAddress<Self::Api>>::None)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::HatomLiquidStaking => call
                .delegate_hatom()
                .egld(payments.get(0).amount.as_big_uint())
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // --- Hatom Lending ---
            types::ActionType::HatomRedeem => call
                .hatom_redeem(OptionalValue::<BigUint<Self::Api>>::None)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::HatomSupply(_) => call
                .hatom_mint()
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
        };

        // Standard result handling for non-add-liquidity operations
        // (add_liquidity is handled at the start of this function via pre_balance_and_add_liquidity)
        let result = back_transfers.into_payment_vec();
        let result_len = result.len();
        for funds in result.iter() {
            if result_len == 1 {
                // For single-output operations, set prev_result for PrevAmount mode
                vault.set_prev_result(&funds);
            }
            vault.deposit(&funds.token_identifier, &funds.amount);
        }
    }

    /// Resolve the proxy address for a given instruction
    fn get_proxy_call(
        &self,
        instr: &Instruction<Self::Api>,
        payments: &ManagedVec<Payment<Self::Api>>,
    ) -> proxies::ProxyTo<Self::Api> {
        let first_payment = payments.get(0).clone();

        match &instr.action {
            types::ActionType::XExchangeSwap(token_out) => {
                self.proxy_call(self.get_pair_x(token_out, unsafe {
                    first_payment.token_identifier.as_esdt_unchecked()
                }))
            }
            types::ActionType::XExchangeAddLiquidity => unsafe {
                let second_token = payments
                    .get(1)
                    .clone()
                    .token_identifier
                    .into_esdt_unchecked();
                self.proxy_call(self.get_pair_x(
                    first_payment.token_identifier.as_esdt_unchecked(),
                    &second_token,
                ))
            },
            types::ActionType::OneDexSwap(_)
            | types::ActionType::OneDexAddLiquidity(_)
            | types::ActionType::OneDexRemoveLiquidity => {
                self.proxy_call(ManagedAddress::from(ONE_DEX_ROUTER))
            }
            types::ActionType::Wrapping | types::ActionType::UnWrapping => {
                self.proxy_call(ManagedAddress::from(WRAPPER_SC))
            }
            types::ActionType::XoxnoLiquidStaking => {
                self.proxy_call(ManagedAddress::from(XEGLD_STAKING))
            }
            types::ActionType::LXoxnoLiquidStaking => {
                self.proxy_call(ManagedAddress::from(LXOXNO_STAKING))
            }
            types::ActionType::HatomLiquidStaking => {
                self.proxy_call(ManagedAddress::from(HATOM_STAKING))
            }
            types::ActionType::HatomRedeem => self.proxy_call(unsafe {
                self.get_hatom_market(first_payment.token_identifier.clone().as_esdt_unchecked())
            }),
            types::ActionType::HatomSupply(token) => self.proxy_call(self.get_hatom_market(token)),
            _ => self.proxy_call(instr.address.clone().unwrap()),
        }
    }

    // --- Pre-Balance Add Liquidity (Optimized ZAP) ---

    /// Check if this action type is a CPMM add liquidity that can be pre-balanced
    fn is_zappable_add_liquidity(&self, action: &types::ActionType<Self::Api>) -> bool {
        matches!(
            action,
            types::ActionType::XExchangeAddLiquidity
                | types::ActionType::OneDexAddLiquidity(_)
                | types::ActionType::JexAddLiquidity
        )
    }

    /// Pre-balance tokens and add liquidity in a single operation
    ///
    /// Instead of: add_liquidity → ZAP leftover → add_liquidity again
    /// This does: compute optimal swap → swap → add_liquidity (once)
    ///
    /// Saves ~400k gas by avoiding the second add_liquidity call
    fn pre_balance_and_add_liquidity(
        &self,
        vault: &mut Vault<Self::Api>,
        instr: &Instruction<Self::Api>,
        payments: &ManagedVec<Payment<Self::Api>>,
    ) {
        let min = BigUint::from(MIN_INTERNAL_OUTPUT);

        // 1. Get pool info
        let pool_address = self.resolve_pool_address(&instr.action, instr, payments);
        let (reserve_first, reserve_second) = self.get_reserves(&instr.action, &pool_address);
        let pool_first_token = self.get_pool_first_token(&instr.action, &pool_address);
        let pool_second_token = self.get_pool_second_token(&instr.action, &pool_address);
        let (fee_num, fee_denom) = self.get_fee(&instr.action, &pool_address);
        let fee_mode = match &instr.action {
            types::ActionType::JexAddLiquidity => zap::FeeMode::OnOutput,
            _ => zap::FeeMode::OnInput,
        };

        // 2. Get current balances (payments are always in first, second order)
        let balance_first = payments.get(0).amount.as_big_uint().clone();
        let balance_second = payments.get(1).amount.as_big_uint().clone();
        let token_first = payments.get(0).token_identifier.clone();
        let token_second = payments.get(1).token_identifier.clone();

        // 3. Compute optimal swap to balance tokens
        let (swap_from_first, swap_amount) = zap::compute_optimal_pre_swap(
            &balance_first,
            &balance_second,
            &reserve_first,
            &reserve_second,
            fee_num,
            fee_denom,
            fee_mode,
        );

        // 4. Execute swap if needed and compute final balances
        let (final_first, final_second) = if swap_amount > 0u64 {
            if swap_from_first {
                // Swap some first token for second
                let swap_payment = ManagedVec::from_single_item(Payment::new(
                    token_first.clone(),
                    0u64,
                    swap_amount.clone().into_non_zero().unwrap(),
                ));

                let swap_result = match &instr.action {
                    types::ActionType::XExchangeAddLiquidity => self
                        .proxy_call(pool_address.clone())
                        .xexchange(&pool_second_token, min.clone())
                        .payment(&swap_payment)
                        .returns(ReturnsBackTransfersReset)
                        .sync_call(),
                    types::ActionType::OneDexAddLiquidity(_) => {
                        let mut path = MultiValueEncoded::new();
                        path.push(pool_first_token.clone());
                        path.push(pool_second_token.clone());
                        self.proxy_call(ManagedAddress::from(ONE_DEX_ROUTER))
                            .onedex(min.clone(), false, path)
                            .payment(&swap_payment)
                            .returns(ReturnsBackTransfersReset)
                            .sync_call()
                    }
                    types::ActionType::JexAddLiquidity => self
                        .proxy_call(pool_address.clone())
                        .jex(min.clone())
                        .payment(&swap_payment)
                        .returns(ReturnsBackTransfersReset)
                        .sync_call(),
                    _ => return,
                };

                let received = swap_result.to_single_esdt().amount;
                (&balance_first - &swap_amount, &balance_second + &received)
            } else {
                // Swap some second token for first
                let swap_payment = ManagedVec::from_single_item(Payment::new(
                    token_second.clone(),
                    0u64,
                    swap_amount.clone().into_non_zero().unwrap(),
                ));

                let swap_result = match &instr.action {
                    types::ActionType::XExchangeAddLiquidity => self
                        .proxy_call(pool_address.clone())
                        .xexchange(&pool_first_token, min.clone())
                        .payment(&swap_payment)
                        .returns(ReturnsBackTransfersReset)
                        .sync_call(),
                    types::ActionType::OneDexAddLiquidity(_) => {
                        let mut path = MultiValueEncoded::new();
                        path.push(pool_second_token.clone());
                        path.push(pool_first_token.clone());
                        self.proxy_call(ManagedAddress::from(ONE_DEX_ROUTER))
                            .onedex(min.clone(), false, path)
                            .payment(&swap_payment)
                            .returns(ReturnsBackTransfersReset)
                            .sync_call()
                    }
                    types::ActionType::JexAddLiquidity => self
                        .proxy_call(pool_address.clone())
                        .jex(min.clone())
                        .payment(&swap_payment)
                        .returns(ReturnsBackTransfersReset)
                        .sync_call(),
                    _ => return,
                };

                let received = swap_result.to_single_esdt().amount;
                (&balance_first + &received, &balance_second - &swap_amount)
            }
        } else {
            // Already balanced, no swap needed
            (balance_first, balance_second)
        };

        // 5. Create balanced payments for add_liquidity (always in first, second order)
        let mut lp_payments = ManagedVec::new();
        lp_payments.push(Payment::new(
            token_first.clone(),
            0u64,
            final_first.into_non_zero().unwrap(),
        ));
        lp_payments.push(Payment::new(
            token_second.clone(),
            0u64,
            final_second.into_non_zero().unwrap(),
        ));

        // 6. Execute SINGLE add_liquidity
        let lp_result = self
            .proxy_call(pool_address)
            .xdex_add_liquidity(min.clone(), min)
            .payment(&lp_payments)
            .returns(ReturnsBackTransfersReset)
            .sync_call();

        // 7. Deposit all results to vault (LP tokens + any minimal dust)
        for payment in lp_result.into_payment_vec().iter() {
            vault.deposit(&payment.token_identifier, &payment.amount);
        }
    }

    /// Resolve pool address for ZAP operations based on action type.
    /// - xExchange: lookup from storage using token pair
    /// - OneDex: use ONE_DEX_ROUTER constant
    /// - Jex: use provided instruction address
    fn resolve_pool_address(
        &self,
        action: &types::ActionType<Self::Api>,
        instr: &Instruction<Self::Api>,
        payments: &ManagedVec<Payment<Self::Api>>,
    ) -> ManagedAddress {
        match action {
            types::ActionType::XExchangeAddLiquidity => {
                // Look up pair address from storage using the two input tokens
                let first_token = unsafe {
                    payments
                        .get(0)
                        .token_identifier
                        .clone()
                        .into_esdt_unchecked()
                };
                let second_token = unsafe {
                    payments
                        .get(1)
                        .token_identifier
                        .clone()
                        .into_esdt_unchecked()
                };
                self.get_pair_x(&first_token, &second_token)
            }
            types::ActionType::OneDexAddLiquidity(_) => {
                // OneDex uses hardcoded router address
                ManagedAddress::from(ONE_DEX_ROUTER)
            }
            types::ActionType::JexAddLiquidity => {
                // Jex requires explicit address from instruction
                instr.address.clone().unwrap()
            }
            _ => instr.address.clone().unwrap_or_else(ManagedAddress::zero),
        }
    }
}
