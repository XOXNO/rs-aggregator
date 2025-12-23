#![no_std]

multiversx_sc::imports!();
multiversx_sc::derive_imports!();

pub mod constants;
pub mod proxies;
pub mod storage;
pub mod types;
pub mod vault;

use constants::{
    HATOM_STAKING, LXOXNO_STAKING, MIN_INTERNAL_OUTPUT, ONE_DEX_ROUTER, WRAPPER_SC, XEGLD_STAKING,
};
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

    // ==========================================================================
    // Main Aggregation Endpoint
    // ==========================================================================

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
        require!(
            vault.has_minimum(&token_out, &min_amount_out),
            "Output amount below minimum"
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

    // ==========================================================================
    // Instruction Execution
    // ==========================================================================

    /// Execute a single instruction by dispatching to the appropriate DEX proxy
    fn execute_instruction(&self, vault: &mut Vault<Self::Api>, instr: &Instruction<Self::Api>) {
        let mut input_payments = ManagedVec::new();

        require!(!instr.inputs.is_empty(), "No inputs in instruction");

        // 1. Withdraw all required inputs from vault
        for input in instr.inputs.iter() {
            let actual_amount = match &input.mode {
                AmountMode::Fixed(amount) => vault.withdraw(&input.token, amount),
                AmountMode::Ppm(ppm) => vault.withdraw_ppm(&input.token, ppm),
                AmountMode::All => vault.withdraw_all(&input.token),
                AmountMode::PrevAmount => {
                    let prev_result = vault.get_prev_result();
                    require!(prev_result.is_some(), "PrevAmount not available");
                    let prev_value = prev_result.as_ref().unwrap();
                    require!(
                        input.token == prev_value.token_identifier,
                        "PrevAmount token mismatch"
                    );
                    vault.withdraw(&input.token, prev_value.amount.clone().as_big_uint())
                }
            };

            require!(actual_amount > 0u64, "Zero input amount");

            input_payments.push(Payment::new(
                input.token.clone(),
                0u64,
                actual_amount.into_non_zero().unwrap(),
            ));
        }

        // 2. Dispatch to appropriate proxy
        self.dispatch_to_proxy(vault, instr, &input_payments);
    }

    // ==========================================================================
    // Dispatch Logic
    // ==========================================================================

    /// Dispatch instruction to the appropriate DEX proxy
    fn dispatch_to_proxy(
        &self,
        vault: &mut Vault<Self::Api>,
        instr: &Instruction<Self::Api>,
        payments: &ManagedVec<Payment<Self::Api>>,
    ) {
        let min = BigUint::from(MIN_INTERNAL_OUTPUT);

        let mut call = self.get_proxy_call(instr, payments);

        // Execute the appropriate proxy call based on DEX type
        let back_transfers = match &instr.action {
            // ═══════════════════════════════════════════════════════════════
            // xExchange
            // ═══════════════════════════════════════════════════════════════
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

            // ═══════════════════════════════════════════════════════════════
            // AshSwap V1 (Stable)
            // ═══════════════════════════════════════════════════════════════
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

            // ═══════════════════════════════════════════════════════════════
            // AshSwap V2 (Crypto)
            // ═══════════════════════════════════════════════════════════════
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

            // ═══════════════════════════════════════════════════════════════
            // OneDex
            // ═══════════════════════════════════════════════════════════════
            types::ActionType::OneDexSwap(token_out) => {
                let mut path = MultiValueEncoded::new();
                for input in instr.inputs.iter() {
                    unsafe {
                        path.push(input.token.clone().into_esdt_unchecked());
                    }
                }
                path.push(token_out.clone());
                call.onedex(min, false, path)
                    .payment(payments)
                    .returns(ReturnsBackTransfersReset)
                    .sync_call()
            }
            types::ActionType::OneDexAddLiquidity => call
                .xdex_add_liquidity(min.clone(), min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::OneDexRemoveLiquidity => call
                .onedex_remove_liquidity(min.clone(), min.clone(), false)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // ═══════════════════════════════════════════════════════════════
            // Jex (CPMM)
            // ═══════════════════════════════════════════════════════════════
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

            // ═══════════════════════════════════════════════════════════════
            // Jex (Stable)
            // ═══════════════════════════════════════════════════════════════
            types::ActionType::JexStableSwap(token_out) => call
                .jex_swap_stable(token_out, min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::JexStableAddLiquidity => call
                .jex_add_liquidity_stable(min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::JexStableRemoveLiquidity => call
                .jex(min)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // ═══════════════════════════════════════════════════════════════
            // EGLD Wrapping
            // ═══════════════════════════════════════════════════════════════
            types::ActionType::Wrapping => call
                .wrap_egld()
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::UnWrapping => call
                .unwrap_egld()
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // ═══════════════════════════════════════════════════════════════
            // Liquid Staking
            // ═══════════════════════════════════════════════════════════════
            types::ActionType::XoxnoLiquidStaking | types::ActionType::LXoxnoLiquidStaking => call
                .delegate(OptionalValue::<multiversx_sc::types::ManagedAddress<Self::Api>>::None)
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),
            types::ActionType::HatomLiquidStaking => call
                .delegate_hatom()
                .payment(payments)
                .returns(ReturnsBackTransfersReset)
                .sync_call(),

            // ═══════════════════════════════════════════════════════════════
            // Hatom Lending
            // ═══════════════════════════════════════════════════════════════
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

        let result = back_transfers.into_payment_vec();
        let result_len = result.len();
        // 3. Deposit result(s) back to vault
        for funds in result.iter() {
            if result_len == 1 {
                // For single-output operations, ensure minimum output
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
            | types::ActionType::OneDexAddLiquidity
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
}
