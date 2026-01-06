use crate::constants::{HATOM_CONTROLLER, ONE_DEX_ROUTER, TOTAL_FEE, XEXCHANGE_ROUTER};
use crate::types::{ActionType, PairFee, PairTokens, ReferralConfig};

multiversx_sc::imports!();

/// Pair reserves (first_reserve, second_reserve)
pub type PairReserves<M> = (BigUint<M>, BigUint<M>);

#[multiversx_sc::module]
pub trait Storage {
    // =========================================================================
    // Unified Reserve & Fee Getters
    // =========================================================================

    /// Get the pool's first token ID for correct payment ordering
    fn get_pool_first_token(
        &self,
        action: &ActionType<Self::Api>,
        pool_address: &ManagedAddress,
    ) -> TokenIdentifier {
        match action {
            ActionType::XExchangeAddLiquidity => {
                self.xexchange_first_token_id(pool_address.clone()).get()
            }
            ActionType::OneDexAddLiquidity(pair_id) => {
                let router = ManagedAddress::from(ONE_DEX_ROUTER);
                self.onedex_first_token_id(router, *pair_id).get()
            }
            ActionType::JexAddLiquidity => self.jex_first_token_id(pool_address.clone()).get(),
            _ => TokenIdentifier::from_esdt_bytes(&[]),
        }
    }

    /// Get the pool's second token ID for swap target
    fn get_pool_second_token(
        &self,
        action: &ActionType<Self::Api>,
        pool_address: &ManagedAddress,
    ) -> TokenIdentifier {
        match action {
            ActionType::XExchangeAddLiquidity => {
                self.xexchange_second_token_id(pool_address.clone()).get()
            }
            ActionType::OneDexAddLiquidity(pair_id) => {
                let router = ManagedAddress::from(ONE_DEX_ROUTER);
                self.onedex_second_token_id(router, *pair_id).get()
            }
            ActionType::JexAddLiquidity => self.jex_second_token_id(pool_address.clone()).get(),
            _ => TokenIdentifier::from_esdt_bytes(&[]),
        }
    }

    /// Get reserves for a pair based on action type (only for add liquidity zap)
    fn get_reserves(
        &self,
        action: &ActionType<Self::Api>,
        pair_address: &ManagedAddress,
    ) -> PairReserves<Self::Api> {
        match action {
            ActionType::XExchangeAddLiquidity => self.get_xexchange_reserves(pair_address),
            ActionType::OneDexAddLiquidity(pair_id) => {
                self.get_onedex_reserves(pair_address, *pair_id)
            }
            ActionType::JexAddLiquidity => self.get_jex_reserves(pair_address),
            // Other actions don't need reserves for zap
            _ => (BigUint::zero(), BigUint::zero()),
        }
    }

    /// Get fee parameters for a pair based on action type (only for add liquidity zap)
    /// Returns (total_fee, special_fee, lp_fee, fee_denom)
    /// - total_fee: used for output calculation
    /// - special_fee: portion that leaves pool (for xExchange)
    /// - lp_fee: portion that stays in pool (for JEX OnOutput mode)
    fn get_fee(
        &self,
        action: &ActionType<Self::Api>,
        pair_address: &ManagedAddress,
    ) -> (u64, u64, u64, u64) {
        match action {
            // xExchange: total_fee_percent with base 100,000
            // special_fee leaves pool (burned/sent to fees collector)
            ActionType::XExchangeAddLiquidity => {
                let total_fee = self.xexchange_total_fee_percent(pair_address.clone()).get();
                let special_fee = self
                    .xexchange_special_fee_percent(pair_address.clone())
                    .get();
                (total_fee, special_fee, 0, 100_000)
            }
            // OneDex: PairFee enum with base 10,000
            // owner_fee + real_yield_fee leave pool, lp_fee stays
            ActionType::OneDexAddLiquidity(pair_id) => {
                let router = ManagedAddress::from(ONE_DEX_ROUTER);
                let pair_fee = self.one_dex_pair_fee(router, *pair_id).get();
                let total = pair_fee.get_total_fee_percentage();
                let special = pair_fee.get_special_fee_percentage();
                (total, special, 0, TOTAL_FEE as u64)
            }
            // JEX: liq_providers_fees + platform_fees with base 10,000
            // Only LP fees stay in pool, platform fees leave (fee-on-output model)
            ActionType::JexAddLiquidity => {
                let lp_fees = self.jex_liq_providers_fees(pair_address.clone()).get();
                let platform_fees = self.jex_platform_fees(pair_address.clone()).get();
                let total_fee = (lp_fees + platform_fees) as u64;
                let lp_fee = lp_fees as u64;
                (total_fee, 0, lp_fee, TOTAL_FEE as u64)
            }
            // Other actions don't need fee for zap
            _ => (0, 0, 0, TOTAL_FEE as u64),
        }
    }

    // =========================================================================
    // xExchange Storage
    // =========================================================================

    #[storage_mapper_from_address("pair_map")]
    fn pair_map(
        &self,
        address: ManagedAddress,
    ) -> MapMapper<PairTokens<Self::Api>, ManagedAddress, ManagedAddress>;

    #[storage_mapper_from_address("reserve")]
    fn xexchange_pair_reserve(
        &self,
        address: ManagedAddress,
        token_id: &TokenIdentifier,
    ) -> SingleValueMapper<BigUint, ManagedAddress>;

    #[storage_mapper_from_address("first_token_id")]
    fn xexchange_first_token_id(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<TokenIdentifier, ManagedAddress>;

    #[storage_mapper_from_address("second_token_id")]
    fn xexchange_second_token_id(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<TokenIdentifier, ManagedAddress>;

    fn get_xexchange_reserves(&self, pair_address: &ManagedAddress) -> PairReserves<Self::Api> {
        let addr = pair_address.clone();
        let first_token = self.xexchange_first_token_id(addr.clone()).get();
        let second_token = self.xexchange_second_token_id(addr.clone()).get();
        (
            self.xexchange_pair_reserve(addr.clone(), &first_token)
                .get(),
            self.xexchange_pair_reserve(addr, &second_token).get(),
        )
    }

    fn get_pair_x(
        &self,
        first_token_id: &TokenIdentifier,
        second_token_id: &TokenIdentifier,
    ) -> ManagedAddress {
        let mapper = self.pair_map(ManagedAddress::from(XEXCHANGE_ROUTER));

        let mut address = mapper
            .get(&PairTokens {
                first_token_id: first_token_id.clone(),
                second_token_id: second_token_id.clone(),
            })
            .unwrap_or_else(ManagedAddress::zero);

        if address.is_zero() {
            address = mapper
                .get(&PairTokens {
                    first_token_id: second_token_id.clone(),
                    second_token_id: first_token_id.clone(),
                })
                .unwrap_or_else(ManagedAddress::zero);
        }
        address
    }

    // =========================================================================
    // OneDex Storage
    // =========================================================================

    #[storage_mapper_from_address("pair_first_token_id")]
    fn onedex_first_token_id(
        &self,
        address: ManagedAddress,
        pair_id: usize,
    ) -> SingleValueMapper<TokenIdentifier, ManagedAddress>;

    #[storage_mapper_from_address("pair_second_token_id")]
    fn onedex_second_token_id(
        &self,
        address: ManagedAddress,
        pair_id: usize,
    ) -> SingleValueMapper<TokenIdentifier, ManagedAddress>;

    #[storage_mapper_from_address("pair_first_token_reserve")]
    fn onedex_first_token_reserve(
        &self,
        address: ManagedAddress,
        pair_id: usize,
    ) -> SingleValueMapper<BigUint, ManagedAddress>;

    #[storage_mapper_from_address("pair_second_token_reserve")]
    fn onedex_second_token_reserve(
        &self,
        address: ManagedAddress,
        pair_id: usize,
    ) -> SingleValueMapper<BigUint, ManagedAddress>;

    fn get_onedex_reserves(
        &self,
        router_address: &ManagedAddress,
        pair_id: usize,
    ) -> PairReserves<Self::Api> {
        let first_reserve = self
            .onedex_first_token_reserve(router_address.clone(), pair_id)
            .get();
        let second_reserve = self
            .onedex_second_token_reserve(router_address.clone(), pair_id)
            .get();

        (first_reserve, second_reserve)
    }

    // =========================================================================
    // Jex Storage
    // =========================================================================

    #[storage_mapper_from_address("first_token_id")]
    fn jex_first_token_id(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<TokenIdentifier, ManagedAddress>;

    #[storage_mapper_from_address("second_token_id")]
    fn jex_second_token_id(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<TokenIdentifier, ManagedAddress>;

    #[storage_mapper_from_address("first_token_reserve")]
    fn jex_first_token_reserve(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<BigUint, ManagedAddress>;

    #[storage_mapper_from_address("second_token_reserve")]
    fn jex_second_token_reserve(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<BigUint, ManagedAddress>;

    fn get_jex_reserves(&self, pair_address: &ManagedAddress) -> PairReserves<Self::Api> {
        let first_reserve = self.jex_first_token_reserve(pair_address.clone()).get();
        let second_reserve = self.jex_second_token_reserve(pair_address.clone()).get();

        (first_reserve, second_reserve)
    }

    // =========================================================================
    // Hatom Storage
    // =========================================================================

    #[storage_mapper_from_address("money_markets")]
    fn money_markets(
        &self,
        address: ManagedAddress,
        token_id: &TokenIdentifier,
    ) -> SingleValueMapper<ManagedAddress, ManagedAddress>;

    fn get_hatom_market(&self, h_token: &TokenIdentifier) -> ManagedAddress {
        self.money_markets(ManagedAddress::from(HATOM_CONTROLLER), h_token)
            .get()
    }

    #[storage_mapper_from_address("total_fee_percent")]
    fn xexchange_total_fee_percent(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<u64, ManagedAddress>;

    #[storage_mapper_from_address("special_fee_percent")]
    fn xexchange_special_fee_percent(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<u64, ManagedAddress>;

    #[storage_mapper_from_address("liq_providers_fees")]
    fn jex_liq_providers_fees(
        &self,
        address: ManagedAddress,
    ) -> SingleValueMapper<u32, ManagedAddress>;

    #[storage_mapper_from_address("platform_fees")]
    fn jex_platform_fees(&self, address: ManagedAddress) -> SingleValueMapper<u32, ManagedAddress>;

    #[storage_mapper_from_address("pair_fee")]
    fn one_dex_pair_fee(
        &self,
        address: ManagedAddress,
        pair_id: usize,
    ) -> SingleValueMapper<PairFee, ManagedAddress>;

    // =========================================================================
    // Fee & Referral Storage (local contract storage)
    // =========================================================================
    #[view(getReferralIdCounter)]
    #[storage_mapper("id")]
    fn referral_id_counter(&self) -> SingleValueMapper<u64>;

    #[view(getReferralConfig)]
    #[storage_mapper("refConfig")]
    fn referral_config(&self, id: u64) -> SingleValueMapper<ReferralConfig<Self::Api>>;

    #[storage_mapper("refBalance")]
    fn referrer_balances(&self, referral_id: u64) -> MapMapper<TokenId, BigUint>;

    #[view(getStaticFee)]
    #[storage_mapper("fee")]
    fn static_fee(&self) -> SingleValueMapper<u32>;

    #[storage_mapper("balances")]
    fn admin_fees(&self) -> MapMapper<TokenId, BigUint>;
}
