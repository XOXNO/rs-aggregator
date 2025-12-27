multiversx_sc::imports!();

use crate::constants::{
    HATOM_STAKING, LXOXNO_STAKING, MIN_INTERNAL_OUTPUT, ONE_DEX_ROUTER, TOTAL_FEE, WRAPPER_SC,
    XEGLD_STAKING,
};
use crate::errors::{
    ERR_PREV_AMOUNT_NOT_AVAILABLE, ERR_PREV_AMOUNT_TOKEN_MISMATCH, ERR_ZERO_INPUT_AMOUNT,
};
use crate::types::{
    AmountMode, CompactAction, CompactMode, InputArg, Instruction, IDX_AUTO, IDX_EGLD, IDX_NONE,
};
use crate::vault::Vault;
use crate::zap;
use crate::{proxies, types};
use multiversx_sc::chain_core::EGLD_000000_TOKEN_IDENTIFIER;

/// Type aliases for compact instruction processing
pub type TokenRegistry<M> = ManagedVec<M, TokenIdentifier<M>>;
pub type AddressRegistry<M> = ManagedVec<M, ManagedAddress<M>>;
pub type AmountRegistry<M> = ManagedVec<M, BigUint<M>>;

/// Utility functions module for aggregator operations
#[multiversx_sc::module]
pub trait Utils: crate::storage::Storage {
    #[proxy]
    fn proxy_call(&self, address: ManagedAddress) -> proxies::Proxy<Self::Api>;

    /// Return only the output token to the caller, keep dust as protocol revenue
    fn return_vault_to_caller(&self, vault: Vault<Self::Api>, token_out: &TokenId<Self::Api>) {
        let caller = self.blockchain().get_caller();

        for payment in vault.get_all_payments().iter() {
            if payment.token_identifier == *token_out {
                // Send only the output token to caller
                self.tx().to(&caller).payment(payment.clone()).transfer();
            } else {
                // Keep all other tokens (dust) as protocol revenue
                self.accumulate_admin_fee(&payment.token_identifier, payment.amount.as_big_uint());
            }
        }
    }

    /// Resolve token index to TokenId (vault format)
    fn resolve_token_to_id(
        &self,
        idx: u8,
        tokens: &TokenRegistry<Self::Api>,
    ) -> TokenId<Self::Api> {
        match idx {
            IDX_EGLD => TokenId::from(EGLD_000000_TOKEN_IDENTIFIER.as_bytes()),
            _ => TokenId::from(tokens.get(idx as usize).as_managed_buffer().clone()),
        }
    }

    /// Decode a compact instruction into a full Instruction struct
    ///
    /// Format: MultiValue6<u8, u8, u8, u8, u8, u16>
    ///
    /// Layout for most actions:
    ///   [action, tok1_idx, mode1, tok2_idx, mode2, addr_idx(u16)]
    ///
    /// Layout for OneDex add liquidity:
    ///   [action, tok1, tok2, shared_mode, 0, pair_id(u16)]
    fn decode_compact_instruction(
        &self,
        action_byte: u8,
        byte1: u8,
        byte2: u8,
        byte3: u8,
        byte4: u8,
        pair_id_or_addr: u16,
        tokens: &TokenRegistry<Self::Api>,
        addresses: &AddressRegistry<Self::Api>,
        amounts: &AmountRegistry<Self::Api>,
    ) -> Instruction<Self::Api> {
        let compact_action = CompactAction::from_u8(action_byte)
            .unwrap_or_else(|| sc_panic!("Invalid action type: {}", action_byte));

        // Build ActionType from compact action
        let action = self.build_action_type(&compact_action, byte1, pair_id_or_addr, tokens);

        // Build inputs based on action type
        let inputs = self.build_inputs(
            &compact_action,
            byte1,
            byte2,
            byte3,
            byte4,
            pair_id_or_addr,
            tokens,
            amounts,
        );

        // Resolve address (for OneDex add liquidity, addr is auto-resolved)
        let address = if compact_action.needs_pair_id() || pair_id_or_addr as u8 == IDX_AUTO {
            None // Auto-resolved in dispatch
        } else {
            Some(addresses.get(pair_id_or_addr as usize).clone())
        };

        Instruction {
            action,
            inputs,
            address,
        }
    }

    /// Build ActionType from CompactAction, resolving output token where needed
    ///
    /// For most actions, byte1 is tok1_idx.
    /// For OneDex add liquidity, pair_id is passed directly as u16.
    fn build_action_type(
        &self,
        compact: &CompactAction,
        byte1: u8,
        pair_id_or_addr: u16,
        tokens: &TokenRegistry<Self::Api>,
    ) -> types::ActionType<Self::Api> {
        match compact {
            CompactAction::XExchangeSwap => {
                let out_token = self.resolve_token(byte1, tokens);
                types::ActionType::XExchangeSwap(out_token)
            }
            CompactAction::XExchangeAddLiquidity => types::ActionType::XExchangeAddLiquidity,
            CompactAction::XExchangeRemoveLiquidity => types::ActionType::XExchangeRemoveLiquidity,
            CompactAction::AshSwapPoolSwap => {
                let out_token = self.resolve_token(byte1, tokens);
                types::ActionType::AshSwapPoolSwap(out_token)
            }
            CompactAction::AshSwapPoolAddLiquidity => types::ActionType::AshSwapPoolAddLiquidity,
            CompactAction::AshSwapPoolRemoveLiquidity => {
                // For remove liquidity, byte1 encodes the output token count
                types::ActionType::AshSwapPoolRemoveLiquidity(byte1 as u32)
            }
            CompactAction::AshSwapV2Swap => types::ActionType::AshSwapV2Swap,
            CompactAction::AshSwapV2AddLiquidity => types::ActionType::AshSwapV2AddLiquidity,
            CompactAction::AshSwapV2RemoveLiquidity => {
                types::ActionType::AshSwapV2RemoveLiquidity(byte1 as u32)
            }
            CompactAction::OneDexSwap => {
                let out_token = self.resolve_token(byte1, tokens);
                types::ActionType::OneDexSwap(out_token)
            }
            CompactAction::OneDexAddLiquidity => {
                // pair_id is directly passed as u16
                types::ActionType::OneDexAddLiquidity(pair_id_or_addr as usize)
            }
            CompactAction::OneDexRemoveLiquidity => types::ActionType::OneDexRemoveLiquidity,
            CompactAction::JexSwap => types::ActionType::JexSwap,
            CompactAction::JexAddLiquidity => types::ActionType::JexAddLiquidity,
            CompactAction::JexRemoveLiquidity => types::ActionType::JexRemoveLiquidity,
            CompactAction::JexStableSwap => {
                let out_token = self.resolve_token(byte1, tokens);
                types::ActionType::JexStableSwap(out_token)
            }
            CompactAction::JexStableAddLiquidity => types::ActionType::JexStableAddLiquidity,
            CompactAction::JexStableRemoveLiquidity => {
                types::ActionType::JexStableRemoveLiquidity(byte1 as u32)
            }
            CompactAction::Wrapping => types::ActionType::Wrapping,
            CompactAction::UnWrapping => types::ActionType::UnWrapping,
            CompactAction::XoxnoLiquidStaking => types::ActionType::XoxnoLiquidStaking,
            CompactAction::LXoxnoLiquidStaking => types::ActionType::LXoxnoLiquidStaking,
            CompactAction::HatomLiquidStaking => types::ActionType::HatomLiquidStaking,
            CompactAction::HatomRedeem => types::ActionType::HatomRedeem,
            CompactAction::HatomSupply => {
                let out_token = self.resolve_token(byte1, tokens);
                types::ActionType::HatomSupply(out_token)
            }
        }
    }

    /// Resolve token from index, handling special values
    fn resolve_token(
        &self,
        idx: u8,
        tokens: &TokenRegistry<Self::Api>,
    ) -> TokenIdentifier<Self::Api> {
        match idx {
            IDX_EGLD => TokenIdentifier::from(EGLD_000000_TOKEN_IDENTIFIER),
            _ => tokens.get(idx as usize).clone(),
        }
    }

    /// Build inputs list from compact instruction bytes
    ///
    /// Format: MultiValue6<u8, u8, u8, u8, u8, u16>
    ///
    /// For swap actions (needs_output_token = true):
    ///   - byte1 = output token (used in ActionType, not here)
    ///   - byte2 = input token index
    ///   - byte3 = input amount mode
    ///   - byte4 = unused
    ///
    /// For multi-input stable pool add_liquidity (3 tokens, shared mode):
    ///   - byte1 = input1 token
    ///   - byte2 = input2 token
    ///   - byte3 = input3 token (or IDX_NONE for 2 inputs)
    ///   - byte4 = shared mode for all inputs
    ///
    /// For dual-input actions (CPMM add liquidity):
    ///   - byte1 = input1 token, byte2 = input1 mode
    ///   - byte3 = input2 token, byte4 = input2 mode
    ///
    /// For OneDex add_liquidity:
    ///   - byte1 = tok1, byte2 = tok2, byte3 = shared_mode, byte4 = 0, u16 = pair_id
    fn build_inputs(
        &self,
        compact_action: &CompactAction,
        byte1: u8,
        byte2: u8,
        byte3: u8,
        byte4: u8,
        _pair_id_or_addr: u16,
        tokens: &TokenRegistry<Self::Api>,
        amounts: &AmountRegistry<Self::Api>,
    ) -> Option<ManagedVec<Self::Api, InputArg<Self::Api>>> {
        // For swap-like actions, byte layout is different:
        // byte1 = output token (handled elsewhere), byte2 = input token, byte3 = input mode
        if compact_action.needs_output_token() {
            let input_token_idx = byte2;
            let input_mode = CompactMode::from_u8(byte3);

            // If mode is Prev and token is IDX_NONE, use prev_result
            if matches!(input_mode, CompactMode::Prev) && input_token_idx == IDX_NONE {
                return None;
            }

            let input_token_buf = self.token_idx_to_buffer(input_token_idx, tokens);
            let amount_mode = self.compact_mode_to_amount_mode(&input_mode, amounts);

            let mut inputs = ManagedVec::new();
            inputs.push(InputArg {
                token: input_token_buf,
                mode: amount_mode,
            });
            return Some(inputs);
        }

        // For stable/multi-asset add_liquidity: tok1, tok2, tok3, shared_mode
        if compact_action.is_multi_input_add_liquidity() {
            let shared_mode = CompactMode::from_u8(byte4);
            let amount_mode = self.compact_mode_to_amount_mode(&shared_mode, amounts);

            let mut inputs = ManagedVec::new();

            // Input 1 (always present)
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(byte1, tokens),
                mode: amount_mode.clone(),
            });

            // Input 2 (byte2 is token2 index)
            if byte2 != IDX_NONE {
                inputs.push(InputArg {
                    token: self.token_idx_to_buffer(byte2, tokens),
                    mode: amount_mode.clone(),
                });
            }

            // Input 3 (byte3 is token3 index)
            if byte3 != IDX_NONE {
                inputs.push(InputArg {
                    token: self.token_idx_to_buffer(byte3, tokens),
                    mode: amount_mode,
                });
            }

            return Some(inputs);
        }

        // For remove liquidity with output count: byte1 = count, byte2 = input token, byte3 = input mode
        // Layout: [action, count, in_tok, in_mode, 0, addr]
        if compact_action.needs_output_count() {
            let input_token_idx = byte2;
            let input_mode = CompactMode::from_u8(byte3);

            if matches!(input_mode, CompactMode::Prev) && input_token_idx == IDX_NONE {
                return None;
            }

            let mut inputs = ManagedVec::new();
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(input_token_idx, tokens),
                mode: self.compact_mode_to_amount_mode(&input_mode, amounts),
            });
            return Some(inputs);
        }

        // For OneDex add liquidity with u16 pair_id and shared mode
        // Layout: [action, tok1, tok2, shared_mode, 0, pair_id(u16)]
        if compact_action.needs_pair_id() {
            let token1_idx = byte1;
            let token2_idx = byte2;
            let shared_mode = CompactMode::from_u8(byte3);
            let amount_mode = self.compact_mode_to_amount_mode(&shared_mode, amounts);

            let mut inputs = ManagedVec::new();
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(token1_idx, tokens),
                mode: amount_mode.clone(),
            });
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(token2_idx, tokens),
                mode: amount_mode,
            });
            return Some(inputs);
        }

        // For standard dual-input actions (CPMM add liquidity)
        // Layout: [action, tok1, mode1, tok2, mode2, addr]
        let compact_mode1 = CompactMode::from_u8(byte2);

        // If mode1 is Prev and token is IDX_NONE, use prev_result
        if matches!(compact_mode1, CompactMode::Prev) && byte1 == IDX_NONE {
            return None;
        }

        let mut inputs = ManagedVec::new();

        // First input
        let token1_buf = self.token_idx_to_buffer(byte1, tokens);
        let amount_mode1 = self.compact_mode_to_amount_mode(&compact_mode1, amounts);

        inputs.push(InputArg {
            token: token1_buf,
            mode: amount_mode1,
        });

        // Second input (if present)
        if byte3 != IDX_NONE {
            let token2_buf = self.token_idx_to_buffer(byte3, tokens);
            let compact_mode2 = CompactMode::from_u8(byte4);
            let amount_mode2 = self.compact_mode_to_amount_mode(&compact_mode2, amounts);

            inputs.push(InputArg {
                token: token2_buf,
                mode: amount_mode2,
            });
        }

        Some(inputs)
    }

    /// Convert token index to ManagedBuffer
    fn token_idx_to_buffer(
        &self,
        idx: u8,
        tokens: &TokenRegistry<Self::Api>,
    ) -> ManagedBuffer<Self::Api> {
        match idx {
            IDX_EGLD => ManagedBuffer::from(EGLD_000000_TOKEN_IDENTIFIER.as_bytes()),
            IDX_NONE => ManagedBuffer::new(),
            _ => tokens.get(idx as usize).as_managed_buffer().clone(),
        }
    }

    /// Convert CompactMode to AmountMode
    /// Both Fixed and Ppm read from amounts registry - Fixed gets exact amount, Ppm gets PPM value
    fn compact_mode_to_amount_mode(
        &self,
        mode: &CompactMode,
        amounts: &AmountRegistry<Self::Api>,
    ) -> AmountMode<Self::Api> {
        match mode {
            CompactMode::All => AmountMode::All,
            CompactMode::Prev => AmountMode::PrevAmount,
            CompactMode::Fixed(idx) => AmountMode::Fixed(amounts.get(*idx as usize).clone()),
            CompactMode::Ppm(idx) => {
                // Read PPM value from amounts registry (stored as BigUint, convert to u32)
                let ppm_value = amounts.get(*idx as usize);
                let ppm_u64 = ppm_value.to_u64().unwrap_or(0);
                AmountMode::Ppm(ppm_u64 as u32)
            }
        }
    }

    // --- Instruction Execution ---

    /// Execute a single instruction by dispatching to the appropriate DEX proxy
    fn execute_instruction(
        &self,
        vault: &mut Vault<Self::Api>,
        instr: &Instruction<Self::Api>,
        token_out: &TokenId<Self::Api>,
    ) {
        let mut input_payments = ManagedVec::new();

        if let Some(inputs) = &instr.inputs {
            // 1. Withdraw all required inputs from vault
            for input in inputs.iter() {
                let token = TokenId::from(input.token.clone());

                let actual_amount = match &input.mode {
                    AmountMode::Fixed(amount) => vault.withdraw(&token, amount),
                    AmountMode::Ppm(ppm) => vault.withdraw_ppm(&token, ppm),
                    AmountMode::All => vault.withdraw_all(&token),
                    AmountMode::PrevAmount => {
                        let prev_result = vault.get_prev_result();
                        require!(prev_result.is_some(), ERR_PREV_AMOUNT_NOT_AVAILABLE);
                        let prev_value = prev_result.as_ref().unwrap();
                        require!(
                            token == prev_value.token_identifier,
                            ERR_PREV_AMOUNT_TOKEN_MISMATCH
                        );
                        vault.withdraw(&token, prev_value.amount.clone().as_big_uint())
                    }
                };

                require!(actual_amount > 0u64, ERR_ZERO_INPUT_AMOUNT);

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
        self.dispatch_to_proxy(vault, instr, &input_payments, token_out);
    }

    // --- Dispatch Logic ---

    /// Dispatch instruction to the appropriate DEX proxy
    fn dispatch_to_proxy(
        &self,
        vault: &mut Vault<Self::Api>,
        instr: &Instruction<Self::Api>,
        payments: &ManagedVec<Payment<Self::Api>>,
        token_out: &TokenId<Self::Api>,
    ) {
        // For zappable add_liquidity actions, use pre-balance optimization
        if self.is_zappable_add_liquidity(&instr.action) {
            return self.pre_balance_and_add_liquidity(vault, instr, payments, token_out);
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
            types::ActionType::JexStableRemoveLiquidity(out_tokens) => call
                .jex_remove_liquidity_stable({
                    let mut mv = MultiValueEncoded::new();
                    for _ in 0..*out_tokens {
                        mv.push(min.clone());
                    }
                    mv
                })
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

    // --- Fee Logic ---

    /// Apply fees to the output token before returning to caller
    /// referral_id = 0 means no referral
    fn apply_fees(
        &self,
        vault: &mut Vault<Self::Api>,
        token_out: &TokenId<Self::Api>,
        referral_id: u64,
    ) {
        let output_balance = vault.balance_of(token_out);

        if referral_id > 0 && !self.referral_config(referral_id).is_empty() {
            let config = self.referral_config(referral_id).get();
            if config.active && config.fee > 0 {
                // Calculate referral fee and matching admin fee
                let referral_fee = &output_balance * config.fee / TOTAL_FEE;
                let admin_fee = referral_fee.clone();
                let total = &referral_fee + &admin_fee;

                // Withdraw total fees from vault
                vault.withdraw(token_out, &total);

                // Accumulate fees
                self.accumulate_referrer_fee(referral_id, token_out, &referral_fee);
                self.accumulate_admin_fee(token_out, &admin_fee);
                return;
            }
        }

        // No valid referral - apply static fee
        let static_fee_bps = self.static_fee().get();
        if static_fee_bps > 0 {
            let fee = &output_balance * static_fee_bps / TOTAL_FEE;
            vault.withdraw(token_out, &fee);
            self.accumulate_admin_fee(token_out, &fee);
        }
    }

    fn accumulate_referrer_fee(
        &self,
        id: u64,
        token: &TokenId<Self::Api>,
        amount: &BigUint<Self::Api>,
    ) {
        let token_id: TokenIdentifier<Self::Api> = token.clone().into();
        let current = self
            .referrer_balances(id)
            .get(&token_id)
            .unwrap_or_default();
        self.referrer_balances(id)
            .insert(token_id, &current + amount);
    }

    fn accumulate_admin_fee(&self, token: &TokenId<Self::Api>, amount: &BigUint<Self::Api>) {
        let token_id: TokenIdentifier<Self::Api> = token.clone().into();
        let current = self.admin_fees().get(&token_id).unwrap_or_default();
        self.admin_fees().insert(token_id, &current + amount);
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
        token_out: &TokenId<Self::Api>,
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

        // 7. Deposit LP tokens to vault, accumulate dust to admin fees
        // LP token is always token_out since add_liquidity is always the last instruction
        for payment in lp_result.into_payment_vec().iter() {
            if payment.token_identifier == *token_out {
                vault.deposit(&payment.token_identifier, &payment.amount);
            } else {
                // Dust from LP creation goes to admin fees
                self.accumulate_admin_fee(&payment.token_identifier, payment.amount.as_big_uint());
            }
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
