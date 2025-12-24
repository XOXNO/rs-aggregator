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
use types::{
    AmountMode, CompactAction, CompactMode, InputArg, Instruction, IDX_AUTO, IDX_EGLD, IDX_NONE,
};
use vault::Vault;

/// Type aliases for compact instruction processing
type TokenRegistry<M> = ManagedVec<M, TokenIdentifier<M>>;
type AddressRegistry<M> = ManagedVec<M, ManagedAddress<M>>;
type AmountRegistry<M> = ManagedVec<M, BigUint<M>>;

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

    /// Execute a sequence of aggregator instructions using compact encoding
    ///
    /// # Compact Format
    /// Each instruction is 6 bytes encoded as MultiValue6<u8,u8,u8,u8,u8,u8>:
    /// - Byte 0: action type (see CompactAction enum)
    /// - Byte 1: token1 index into tokens registry (or IDX_EGLD for EGLD, IDX_NONE for prev)
    /// - Byte 2: mode1 (0=All, 1=Prev, 2-127=Fixed amounts[n], 128-255=PPM amounts[n])
    /// - Byte 3: token2 index (or IDX_NONE for single input)
    /// - Byte 4: mode2 (or 0 if single input)
    /// - Byte 5: address index (or IDX_AUTO for auto-resolved addresses)
    ///
    /// # Arguments
    /// * `min_amount_out` - Minimum expected output amount (slippage protection)
    /// * `token_out` - Output token index into tokens registry (or IDX_EGLD for EGLD)
    /// * `referral_id` - Referral ID for fee sharing (0 = no referral)
    /// * `tokens` - Token registry (referenced by index in instructions and token_out)
    /// * `addresses` - Address registry (referenced by index in instructions)
    /// * `amounts` - Values registry (Fixed amounts or PPM values, referenced by mode)
    /// * `instructions` - Compact 6-byte instructions
    ///
    /// # Returns
    /// All remaining vault tokens are sent back to caller
    #[payable("*")]
    #[endpoint(xo)]
    #[allow_multiple_var_args]
    fn aggregate(
        &self,
        min_amount_out: BigUint<Self::Api>,
        token_out: u8,
        referral_id: u64,
        tokens: MultiValueEncodedCounted<TokenIdentifier<Self::Api>>,
        addresses: MultiValueEncodedCounted<ManagedAddress<Self::Api>>,
        amounts: MultiValueEncodedCounted<BigUint<Self::Api>>,
        instructions: MultiValueEncoded<MultiValue6<u8, u8, u8, u8, u8, u8>>,
    ) {
        // 1. Initialize vault from incoming payments
        let payment = self.call_value().single();
        let mut vault = Vault::from_payment(payment);

        // 2. Build registries for O(1) index lookup
        let token_registry: TokenRegistry<Self::Api> = tokens.to_vec();
        let address_registry: AddressRegistry<Self::Api> = addresses.to_vec();
        let amount_registry: AmountRegistry<Self::Api> = amounts.to_vec();

        // Resolve token_out from index
        let token_out_id = self.resolve_token_to_id(token_out, &token_registry);

        // 3. Execute each compact instruction sequentially
        for compact_instr in instructions {
            let (action_byte, tok1_idx, mode1, tok2_idx, mode2, addr_idx) =
                compact_instr.into_tuple();

            // Decode instruction from compact format
            let instruction = self.decode_compact_instruction(
                action_byte,
                tok1_idx,
                mode1,
                tok2_idx,
                mode2,
                addr_idx,
                &token_registry,
                &address_registry,
                &amount_registry,
            );

            self.execute_instruction(&mut vault, &instruction, &token_out_id);
        }

        // 4. Verify minimum output amount
        let current_balance = vault.balance_of(&token_out_id);

        require!(
            vault.has_minimum(&token_out_id, &min_amount_out),
            "Slippage limit exceeded: have {}, need {}",
            current_balance,
            min_amount_out
        );

        // 5. Apply fees before returning (0 = no referral)
        self.apply_fees(&mut vault, &token_out_id, referral_id);

        // 6. Return all vault contents to caller
        self.return_vault_to_caller(vault);
    }

    /// Return all vault contents to the caller
    fn return_vault_to_caller(&self, vault: Vault<Self::Api>) {
        let caller = self.blockchain().get_caller();
        let output_payments = vault.get_all_payments();

        self.tx()
            .to(caller)
            .payment(output_payments)
            .transfer_if_not_empty();
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

    /// Decode a compact 6-byte instruction into a full Instruction struct
    fn decode_compact_instruction(
        &self,
        action_byte: u8,
        tok1_idx: u8,
        mode1: u8,
        tok2_idx: u8,
        mode2: u8,
        addr_idx: u8,
        tokens: &TokenRegistry<Self::Api>,
        addresses: &AddressRegistry<Self::Api>,
        amounts: &AmountRegistry<Self::Api>,
    ) -> Instruction<Self::Api> {
        let compact_action = CompactAction::from_u8(action_byte)
            .unwrap_or_else(|| sc_panic!("Invalid action type: {}", action_byte));

        // Build ActionType from compact action
        let action = self.build_action_type(&compact_action, tok1_idx, tokens);

        // Build inputs based on action type
        let inputs = self.build_inputs(
            &compact_action,
            tok1_idx,
            mode1,
            tok2_idx,
            mode2,
            addr_idx,
            tokens,
            amounts,
        );

        // Resolve address
        let address = if addr_idx == IDX_AUTO {
            None // Auto-resolved in dispatch
        } else {
            Some(addresses.get(addr_idx as usize).clone())
        };

        Instruction {
            action,
            inputs,
            address,
        }
    }

    /// Build ActionType from CompactAction, resolving output token where needed
    fn build_action_type(
        &self,
        compact: &CompactAction,
        tok1_idx: u8,
        tokens: &TokenRegistry<Self::Api>,
    ) -> types::ActionType<Self::Api> {
        match compact {
            CompactAction::XExchangeSwap => {
                let out_token = self.resolve_token(tok1_idx, tokens);
                types::ActionType::XExchangeSwap(out_token)
            }
            CompactAction::XExchangeAddLiquidity => types::ActionType::XExchangeAddLiquidity,
            CompactAction::XExchangeRemoveLiquidity => types::ActionType::XExchangeRemoveLiquidity,
            CompactAction::AshSwapPoolSwap => {
                let out_token = self.resolve_token(tok1_idx, tokens);
                types::ActionType::AshSwapPoolSwap(out_token)
            }
            CompactAction::AshSwapPoolAddLiquidity => types::ActionType::AshSwapPoolAddLiquidity,
            CompactAction::AshSwapPoolRemoveLiquidity => {
                // For remove liquidity, tok1_idx encodes the output token count
                types::ActionType::AshSwapPoolRemoveLiquidity(tok1_idx as u32)
            }
            CompactAction::AshSwapV2Swap => types::ActionType::AshSwapV2Swap,
            CompactAction::AshSwapV2AddLiquidity => types::ActionType::AshSwapV2AddLiquidity,
            CompactAction::AshSwapV2RemoveLiquidity => {
                types::ActionType::AshSwapV2RemoveLiquidity(tok1_idx as u32)
            }
            CompactAction::OneDexSwap => {
                let out_token = self.resolve_token(tok1_idx, tokens);
                types::ActionType::OneDexSwap(out_token)
            }
            CompactAction::OneDexAddLiquidity => {
                // tok1_idx encodes the pair_id for OneDex
                types::ActionType::OneDexAddLiquidity(tok1_idx as usize)
            }
            CompactAction::OneDexRemoveLiquidity => types::ActionType::OneDexRemoveLiquidity,
            CompactAction::JexSwap => types::ActionType::JexSwap,
            CompactAction::JexAddLiquidity => types::ActionType::JexAddLiquidity,
            CompactAction::JexRemoveLiquidity => types::ActionType::JexRemoveLiquidity,
            CompactAction::JexStableSwap => {
                let out_token = self.resolve_token(tok1_idx, tokens);
                types::ActionType::JexStableSwap(out_token)
            }
            CompactAction::JexStableAddLiquidity => types::ActionType::JexStableAddLiquidity,
            CompactAction::JexStableRemoveLiquidity => types::ActionType::JexStableRemoveLiquidity,
            CompactAction::Wrapping => types::ActionType::Wrapping,
            CompactAction::UnWrapping => types::ActionType::UnWrapping,
            CompactAction::XoxnoLiquidStaking => types::ActionType::XoxnoLiquidStaking,
            CompactAction::LXoxnoLiquidStaking => types::ActionType::LXoxnoLiquidStaking,
            CompactAction::HatomLiquidStaking => types::ActionType::HatomLiquidStaking,
            CompactAction::HatomRedeem => types::ActionType::HatomRedeem,
            CompactAction::HatomSupply => {
                let out_token = self.resolve_token(tok1_idx, tokens);
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
    /// For swap actions (needs_output_token = true):
    ///   - tok1_idx = output token (used in ActionType, not here)
    ///   - mode1 = input token index
    ///   - tok2_idx = input amount mode
    ///   - mode2 = unused
    ///
    /// For multi-input stable pool add_liquidity (3 tokens, shared mode):
    ///   - tok1_idx = input1 token
    ///   - mode1 = input2 token
    ///   - tok2_idx = input3 token (or IDX_NONE for 2 inputs)
    ///   - mode2 = shared mode for all inputs
    ///
    /// For dual-input actions (CPMM add liquidity):
    ///   - tok1_idx = input1 token, mode1 = input1 mode
    ///   - tok2_idx = input2 token, mode2 = input2 mode
    ///
    /// For single-input actions:
    ///   - tok1_idx = input token, mode1 = mode
    ///   - tok2_idx = IDX_NONE
    ///
    /// For OneDex add_liquidity with pair_id:
    ///   - tok1_idx = pair_id, mode1 = tok1, tok2_idx = mode1, mode2 = tok2, addr_idx = mode2
    fn build_inputs(
        &self,
        compact_action: &CompactAction,
        tok1_idx: u8,
        mode1: u8,
        tok2_idx: u8,
        mode2: u8,
        addr_idx: u8,
        tokens: &TokenRegistry<Self::Api>,
        amounts: &AmountRegistry<Self::Api>,
    ) -> Option<ManagedVec<Self::Api, InputArg<Self::Api>>> {
        // For swap-like actions, byte layout is different:
        // tok1_idx = output token (handled elsewhere), mode1 = input token, tok2_idx = input mode
        if compact_action.needs_output_token() {
            let input_token_idx = mode1;
            let input_mode = CompactMode::from_u8(tok2_idx);

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
            let shared_mode = CompactMode::from_u8(mode2);
            let amount_mode = self.compact_mode_to_amount_mode(&shared_mode, amounts);

            let mut inputs = ManagedVec::new();

            // Input 1 (always present)
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(tok1_idx, tokens),
                mode: amount_mode.clone(),
            });

            // Input 2 (mode1 is actually token2 index)
            if mode1 != IDX_NONE {
                inputs.push(InputArg {
                    token: self.token_idx_to_buffer(mode1, tokens),
                    mode: amount_mode.clone(),
                });
            }

            // Input 3 (tok2_idx is actually token3 index)
            if tok2_idx != IDX_NONE {
                inputs.push(InputArg {
                    token: self.token_idx_to_buffer(tok2_idx, tokens),
                    mode: amount_mode,
                });
            }

            return Some(inputs);
        }

        // For remove liquidity with output count: tok1_idx = count, mode1 = input token, tok2_idx = input mode
        // Layout: [action, count, in_tok, in_mode, 0, addr]
        if compact_action.needs_output_count() {
            let input_token_idx = mode1;
            let input_mode = CompactMode::from_u8(tok2_idx);

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

        // For OneDex add liquidity with pair_id: tok1_idx = pair_id, inputs in bytes 2-5
        // Layout: [action, pair_id, tok1, mode1, tok2, mode2]
        if compact_action.needs_pair_id() {
            let token1_idx = mode1; // byte 2
            let mode1_value = CompactMode::from_u8(tok2_idx); // byte 3
            let token2_idx = mode2; // byte 4
            let mode2_value = CompactMode::from_u8(addr_idx); // byte 5

            let mut inputs = ManagedVec::new();
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(token1_idx, tokens),
                mode: self.compact_mode_to_amount_mode(&mode1_value, amounts),
            });
            inputs.push(InputArg {
                token: self.token_idx_to_buffer(token2_idx, tokens),
                mode: self.compact_mode_to_amount_mode(&mode2_value, amounts),
            });
            return Some(inputs);
        }

        // For standard dual-input actions (CPMM add liquidity)
        let compact_mode1 = CompactMode::from_u8(mode1);

        // If mode1 is Prev and token is IDX_NONE, use prev_result
        if matches!(compact_mode1, CompactMode::Prev) && tok1_idx == IDX_NONE {
            return None;
        }

        let mut inputs = ManagedVec::new();

        // First input
        let token1_buf = self.token_idx_to_buffer(tok1_idx, tokens);
        let amount_mode1 = self.compact_mode_to_amount_mode(&compact_mode1, amounts);

        inputs.push(InputArg {
            token: token1_buf,
            mode: amount_mode1,
        });

        // Second input (if present)
        if tok2_idx != IDX_NONE {
            let token2_buf = self.token_idx_to_buffer(tok2_idx, tokens);
            let compact_mode2 = CompactMode::from_u8(mode2);
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

    #[proxy]
    fn proxy_call(&self, address: ManagedAddress) -> proxies::Proxy<Self::Api>;

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
                let referral_fee = &output_balance * config.fee / 10_000u32;
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
            let fee = &output_balance * static_fee_bps / 10_000u32;
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

    // --- Admin Endpoints ---

    /// Add a new referral with the given owner and fee
    /// Returns the new referral ID
    #[only_owner]
    #[endpoint(addReferral)]
    fn add_referral(&self, owner: ManagedAddress, fee: u32) -> u64 {
        require!(fee <= 10_000, "Fee exceeds 100%");
        let id = self.referral_id_counter().update(|c| {
            *c += 1;
            *c
        });
        self.referral_config(id).set(types::ReferralConfig {
            owner,
            fee,
            active: true,
        });
        id
    }

    /// Update the fee for an existing referral
    #[only_owner]
    #[endpoint(setReferralFee)]
    fn set_referral_fee(&self, id: u64, fee: u32) {
        require!(!self.referral_config(id).is_empty(), "Referral not found");
        require!(fee <= 10_000, "Fee exceeds 100%");
        self.referral_config(id).update(|c| c.fee = fee);
    }

    /// Enable or disable a referral
    #[only_owner]
    #[endpoint(setReferralActive)]
    fn set_referral_active(&self, id: u64, active: bool) {
        require!(!self.referral_config(id).is_empty(), "Referral not found");
        self.referral_config(id).update(|c| c.active = active);
    }

    /// Set the static fee for trades without a referral
    #[only_owner]
    #[endpoint(setStaticFee)]
    fn set_static_fee(&self, fee: u32) {
        require!(fee <= 10_000, "Fee exceeds 100%");
        self.static_fee().set(fee);
    }

    // --- Claim Endpoints ---

    /// Claim accumulated referral fees for a given referral ID
    /// Can only be called by the referral owner
    #[endpoint(claimReferralFees)]
    fn claim_referral_fees(&self, referral_id: u64) {
        require!(
            !self.referral_config(referral_id).is_empty(),
            "Referral not found"
        );
        let config = self.referral_config(referral_id).get();
        let caller = self.blockchain().get_caller();
        require!(caller == config.owner, "Not referral owner");

        let mut payments = ManagedVec::new();
        for (token, amount) in self.referrer_balances(referral_id).iter() {
            if amount > 0u64 {
                payments.push(EsdtTokenPayment::new(token.clone(), 0, amount));
            }
        }

        // Clear all balances
        self.referrer_balances(referral_id).clear();

        if !payments.is_empty() {
            self.tx().to(&config.owner).payment(&payments).transfer();
        }
    }

    /// Claim accumulated admin fees
    /// Can only be called by the contract owner
    #[only_owner]
    #[endpoint(claimAdminFees)]
    fn claim_admin_fees(&self, recipient: ManagedAddress) {
        let mut payments = ManagedVec::new();
        for (token, amount) in self.admin_fees().iter() {
            if amount > 0u64 {
                payments.push(EsdtTokenPayment::new(token.clone(), 0, amount));
            }
        }

        // Clear all balances
        self.admin_fees().clear();

        if !payments.is_empty() {
            self.tx().to(&recipient).payment(&payments).transfer();
        }
    }

    // --- View Functions ---

    /// Get all accumulated balances for a referrer
    #[view(getReferrerBalances)]
    fn get_referrer_balances(
        &self,
        referral_id: u64,
    ) -> MultiValueEncoded<(TokenIdentifier, BigUint)> {
        let mut result = MultiValueEncoded::new();
        for (token, amount) in self.referrer_balances(referral_id).iter() {
            result.push((token, amount));
        }
        result
    }

    /// Get all accumulated admin fees
    #[view(getAdminFees)]
    fn get_admin_fees_view(&self) -> MultiValueEncoded<(TokenIdentifier, BigUint)> {
        let mut result = MultiValueEncoded::new();
        for (token, amount) in self.admin_fees().iter() {
            result.push((token, amount));
        }
        result
    }
}
