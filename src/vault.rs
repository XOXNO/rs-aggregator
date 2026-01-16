multiversx_sc::imports!();

use crate::errors::{
    ERR_INSUFFICIENT_BALANCE_PREFIX, ERR_ONLY_FUNGIBLE_PREFIX, ERR_TOKEN_NOT_FOUND_PREFIX,
};
use multiversx_sc::api::VMApi;

/// In-memory vault for tracking intermediate token balances during aggregation
/// Uses ManagedMapEncoded for O(1) key-value access
pub struct Vault<M: VMApi> {
    balances: ManagedMapEncoded<M, TokenId<M>, BigUint<M>>,
    tokens: ManagedVec<M, TokenId<M>>,
    prev_result: Option<Payment<M>>,
}

impl<M: VMApi> Vault<M> {
    /// Create a new empty vault
    pub fn new() -> Self {
        Self {
            balances: ManagedMapEncoded::new(),
            tokens: ManagedVec::new(),
            prev_result: None,
        }
    }

    pub fn get_prev_result(&self) -> &Option<Payment<M>> {
        &self.prev_result
    }

    pub fn set_prev_result(&mut self, payment: &Payment<M>) {
        self.prev_result = Some(payment.clone());
    }

    /// Initialize vault from a single ESDT payment
    pub fn from_payment(payments: &PaymentVec<M>) -> Self {
        let mut vault = Self::new();
        for payment in payments.iter() {
            if payment.token_nonce != 0 {
                let mut buffer = ManagedBufferBuilder::<M>::new_from_slice(ERR_ONLY_FUNGIBLE_PREFIX);
                buffer.append_managed_buffer(payment.token_identifier.as_managed_buffer());
                let msg = buffer.into_managed_buffer();
                M::error_api_impl().signal_error_from_buffer(msg.get_handle());
            }
            vault.deposit(&payment.token_identifier, &payment.amount.clone());
        }
        vault
    }

    /// Get balance of a token (panics if not found)
    pub fn balance_of(&self, token: &TokenId<M>) -> BigUint<M> {
        if !self.balances.contains(token) {
            let mut buffer = ManagedBufferBuilder::<M>::new_from_slice(ERR_TOKEN_NOT_FOUND_PREFIX);
            buffer.append_managed_buffer(token.as_managed_buffer());
            let msg = buffer.into_managed_buffer();
            M::error_api_impl().signal_error_from_buffer(msg.get_handle());
        }
        self.balances.get(token)
    }

    /// Add amount to vault (creates entry if token not present)
    pub fn deposit(&mut self, token: &TokenId<M>, amount: &NonZeroBigUint<M>) {
        if !self.balances.contains(token) {
            self.tokens.push(token.clone());
            self.balances.put(token, amount.as_big_uint());
        } else {
            let current = self.balances.get(token);
            self.balances.put(token, &(current + amount.as_big_uint()));
        }
    }

    /// Remove specified amount from vault
    /// Signals error if insufficient balance
    pub fn withdraw(&mut self, token: &TokenId<M>, amount: &BigUint<M>) -> BigUint<M> {
        let current = self.balance_of(token);
        if &current < amount {
            // Build detailed error: "Insufficient vault balance for token X: have Y, need Z"
            let mut buffer =
                ManagedBufferBuilder::<M>::new_from_slice(ERR_INSUFFICIENT_BALANCE_PREFIX);
            buffer.append_managed_buffer(token.as_managed_buffer());
            buffer.append_managed_buffer(&ManagedBuffer::from(b": have "));
            buffer.append_managed_buffer(&current.to_display());
            buffer.append_managed_buffer(&ManagedBuffer::from(b", need "));
            buffer.append_managed_buffer(&amount.to_display());

            let msg = buffer.into_managed_buffer();
            M::error_api_impl().signal_error_from_buffer(msg.get_handle());
        }

        let new_balance = current - amount;
        if new_balance == 0u64 {
            self.remove_token_entry(token);
        } else {
            self.balances.put(token, &new_balance);
        }

        amount.clone()
    }

    /// Withdraw entire balance of a token (panics if not found)
    pub fn withdraw_all(&mut self, token: &TokenId<M>) -> BigUint<M> {
        let amount = self.balance_of(token);
        if amount > 0u64 {
            self.remove_token_entry(token);
        }
        amount
    }

    /// Withdraw a percentage (PPM) of the token balance
    pub fn withdraw_ppm(&mut self, token: &TokenId<M>, ppm: &u32) -> BigUint<M> {
        let amount = self.ppm_of(token, ppm);
        if amount > 0u64 {
            self.withdraw(token, &amount)
        } else {
            BigUint::zero()
        }
    }

    /// Internal helper to remove token from tracking.
    ///
    /// Note: This uses O(N) linear scan to find and remove the token from the list.
    /// This is acceptable because in typical aggregation paths, the number of unique
    /// tokens rarely exceeds 5-10, making the overhead negligible.
    fn remove_token_entry(&mut self, token: &TokenId<M>) {
        // Remove from map - O(1)
        self.balances.remove(token);

        // Remove from list - O(N) where N is number of unique tokens in vault
        let mut index_to_remove = None;
        for (i, t) in self.tokens.iter().enumerate() {
            if t.as_managed_buffer() == token.as_managed_buffer() {
                index_to_remove = Some(i);
                break;
            }
        }

        if let Some(index) = index_to_remove {
            self.tokens.remove(index);
        }
    }

    /// Calculate PPM (parts per million) of vault balance
    /// PPM must be <= 1_000_000 (100%)
    pub fn ppm_of(&self, token: &TokenId<M>, ppm: &u32) -> BigUint<M> {
        // Validate PPM range (should be caught earlier, but defense in depth)
        if *ppm > 1_000_000 {
            M::error_api_impl().signal_error(b"PPM exceeds 1,000,000 (100%)");
        }
        let balance = self.balance_of(token);
        (&balance * *ppm) / 1_000_000u64
    }

    /// Get all non-zero token entries for returning to caller
    pub fn get_all_payments(&self) -> ManagedVec<M, Payment<M>> {
        let mut payments = ManagedVec::new();
        // Read directly from tokens list which is kept in sync
        for token in self.tokens.iter() {
            let amount = self.balance_of(&token);
            payments.push(Payment::new(
                token.clone_value(),
                0u64,
                amount.into_non_zero().unwrap(),
            ));
        }
        payments
    }

    /// Check if vault has at least the minimum amount of a token
    pub fn has_minimum(&self, token: &TokenId<M>, min_amount: &BigUint<M>) -> bool {
        self.balance_of(token) >= *min_amount
    }
}

impl<M: VMApi> Default for Vault<M> {
    fn default() -> Self {
        Self::new()
    }
}
