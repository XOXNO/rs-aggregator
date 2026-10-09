#![no_std]

multiversx_sc::imports!();
multiversx_sc::derive_imports!();

pub mod config;
pub mod constants;
pub mod errors;
pub mod events;
pub mod proxies;
pub mod storage;
pub mod types;
pub mod utils;
pub mod vault;
pub mod zap;

use utils::{AddressRegistry, AmountRegistry, TokenRegistry};
use vault::Vault;

/// MultiversX DEX Aggregator with LP Support
///
/// Executes swap paths from the arb-algo aggregator, supporting:
/// - Token to Token swaps with splits and hops
/// - Token to LP minting
/// - LP to Token burning
/// - LP to LP conversion
#[multiversx_sc::contract]
pub trait Aggregator: storage::Storage + config::Config + utils::Utils + events::Events {
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
        instructions: MultiValueEncoded<MultiValue6<u8, u8, u8, u8, u8, u16>>,
    ) {
        // 1. Initialize vault from incoming payments
        let payment = self.call_value().all();
        let mut vault = Vault::from_payment(&payment);

        // 2. Build registries for O(1) index lookup
        let token_registry: TokenRegistry<Self::Api> = tokens.to_vec();
        let address_registry: AddressRegistry<Self::Api> = addresses.to_vec();
        let amount_registry: AmountRegistry<Self::Api> = amounts.to_vec();

        // Resolve token_out from index
        let token_out_id = self.resolve_token_to_id(token_out, &token_registry);

        // 3. Determine fee direction and apply fee on input if needed
        // Bulk payments (len > 1): always fee on output
        // Single payment: check whitelist rules
        let is_bulk = payment.len() > 1;
        let fee_on_input = if is_bulk {
            false // Bulk payments always fee on output
        } else {
            // Single input: check whitelist rules
            let token_in_id = payment.get(0).token_identifier.clone();
            let in_whitelisted = self.whitelisted_tokens().contains(&token_in_id);
            let out_whitelisted = self.whitelisted_tokens().contains(&token_out_id);
            // Fee on output ONLY if token_out is whitelisted AND token_in is NOT
            // Otherwise fee on input
            !out_whitelisted || in_whitelisted
        };

        let fee_token = if fee_on_input {
            payment.get(0).token_identifier.clone()
        } else {
            token_out_id.clone()
        };
        let mut applied_fees = (0, BigUint::zero(), BigUint::zero());
        let mut admin_fees = Vault::new();

        // Apply fee on INPUT token BEFORE processing instructions
        // This reduces the vault balance before swaps execute
        if fee_on_input && !is_bulk {
            applied_fees = self.apply_fees_on_token(&mut vault, &fee_token, referral_id);
        }

        // 4. Execute each compact instruction sequentially
        for compact_instr in instructions {
            let (action_byte, byte1, byte2, byte3, byte4, pair_id_or_addr) =
                compact_instr.into_tuple();

            // Decode instruction from compact format
            let instruction = self.decode_compact_instruction(
                action_byte,
                byte1,
                byte2,
                byte3,
                byte4,
                pair_id_or_addr,
                &token_registry,
                &address_registry,
                &amount_registry,
            );

            self.execute_instruction(&mut vault, &instruction, &token_out_id, &mut admin_fees);
        }

        // 5. Apply fee on OUTPUT token AFTER processing instructions (if not already applied on input)
        if !fee_on_input {
            applied_fees = self.apply_fees_on_token(&mut vault, &fee_token, referral_id);
        }

        // 6. Verify minimum output amount AFTER fees
        let current_balance = vault.balance_of(&token_out_id);

        require!(
            vault.has_minimum(&token_out_id, &min_amount_out),
            "Slippage limit exceeded: have {}, need {}",
            current_balance,
            min_amount_out
        );

        // 7. Return only output token to caller, keep dust as protocol revenue
        let outputs = self.return_vault_to_caller(vault, &token_out_id, &mut admin_fees);
        let (used_referral_id, admin_fee, referral_fee) = applied_fees;
        if let Some(amount) = admin_fee.into_non_zero() {
            admin_fees.deposit(&fee_token, &amount);
        }
        let mut referral_fees = ManagedVec::new();
        if let Some(amount) = referral_fee.into_non_zero() {
            referral_fees.push(Payment::new(fee_token, 0, amount));
        }

        self.aggregate_event(
            &self.blockchain().get_caller(),
            used_referral_id,
            &payment,
            &outputs,
            &admin_fees.get_all_payments(),
            &referral_fees,
        );
    }
}
