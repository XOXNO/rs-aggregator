multiversx_sc::imports!();

/// In-memory vault for tracking intermediate token balances during aggregation
/// Uses ManagedMapEncoded for O(1) key-value access
pub struct Vault<M: ManagedTypeApi> {
    balances: ManagedMapEncoded<M, TokenId<M>, BigUint<M>>,
    tokens: ManagedVec<M, TokenId<M>>,
    prev_result: Option<Payment<M>>,
}

impl<M: ManagedTypeApi> Vault<M> {
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

    /// Initialize vault from incoming ESDT payments
    pub fn from_payments(payments: &ManagedVec<M, Payment<M>>) -> Self {
        let mut vault = Self::new();
        for payment in payments.iter() {
            if payment.token_nonce != 0 {
                panic!("Only fungible ESDT tokens are accepted");
            }
            // deposit handles tokens list management now
            vault.deposit(&payment.token_identifier, &payment.amount);
        }
        vault
    }

    pub fn from_payment(payment: Ref<Payment<M>>) -> Self {
        let mut vault = Self::new();
        if payment.token_nonce != 0 {
            panic!("Only fungible ESDT tokens are accepted");
        }
        // deposit handles tokens list management now
        vault.deposit(&payment.token_identifier, &payment.amount);
        vault
    }

    /// Get balance of a token (returns 0 if not found)
    pub fn balance_of(&self, token: &TokenId<M>) -> BigUint<M> {
        if !self.balances.contains(token) {
            return BigUint::zero();
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
    /// Panics if insufficient balance
    pub fn withdraw(&mut self, token: &TokenId<M>, amount: &BigUint<M>) -> BigUint<M> {
        let current = self.balance_of(token);
        if &current < amount {
            panic!(
                "Insufficient vault balance for token {}",
                token.as_managed_buffer()
            );
        }

        let new_balance = current - amount;
        if new_balance == 0u64 {
            self.remove_token_entry(token);
        } else {
            self.balances.put(token, &new_balance);
        }

        amount.clone()
    }

    /// Withdraw entire balance of a token
    /// Returns 0 if token not found
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
    pub fn ppm_of(&self, token: &TokenId<M>, ppm: &u32) -> BigUint<M> {
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

impl<M: ManagedTypeApi> Default for Vault<M> {
    fn default() -> Self {
        Self::new()
    }
}
