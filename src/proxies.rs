multiversx_sc::imports!();

#[multiversx_sc::proxy]
pub trait DexProxy {
    // ═══════════════════════════════════════════════════════════════════════════
    // EGLD Wrapping
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("EGLD")]
    #[endpoint(wrapEgld)]
    fn wrap_egld(&self);

    #[payable("*")]
    #[endpoint(unwrapEgld)]
    fn unwrap_egld(&self);

    // ═══════════════════════════════════════════════════════════════════════════
    // xExchange (CPMM)
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(swapTokensFixedInput)]
    fn xexchange(&self, token_out: TokenIdentifier, amount_out_min: BigUint);

    #[payable("*")]
    #[endpoint(addLiquidity)]
    fn xdex_add_liquidity(&self, first_token_amount_min: BigUint, second_token_amount_min: BigUint);

    #[payable("*")]
    #[endpoint(removeLiquidity)]
    fn xdex_remove_liquidity(
        &self,
        first_token_amount_min: BigUint,
        second_token_amount_min: BigUint,
    );

    // ═══════════════════════════════════════════════════════════════════════════
    // OneDex
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(swapMultiTokensFixedInput)]
    fn onedex(
        &self,
        amount_out_min: BigUint,
        unwrap_required: bool,
        path_args: MultiValueEncoded<TokenIdentifier>,
    );

    #[payable("*")]
    #[endpoint(removeLiquidity)]
    fn onedex_remove_liquidity(
        &self,
        first_token_amount_min: BigUint,
        second_token_amount_min: BigUint,
        unwrap_required: bool,
    );

    // ═══════════════════════════════════════════════════════════════════════════
    // Jex (CPMM)
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(swapTokensFixedInput)]
    fn jex(&self, min_amount_out: BigUint);

    // ═══════════════════════════════════════════════════════════════════════════
    // Jex (Stable)
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(swap)]
    fn jex_swap_stable(&self, token_out: TokenIdentifier, amount_out_min: BigUint);

    #[payable("*")]
    #[endpoint(addLiquidity)]
    fn jex_add_liquidity_stable(&self, min_shares: BigUint);

    // ═══════════════════════════════════════════════════════════════════════════
    // AshSwap V1 (StableSwap)
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(exchange)]
    fn ash_exchange_stable(&self, token_out: TokenIdentifier, amount_out_min: BigUint);

    #[payable("*")]
    #[endpoint(addLiquidity)]
    fn ash_add_liquidity_stable(
        &self,
        mint_amount_min: BigUint,
        lp_token_receiver: &ManagedAddress,
    );

    #[payable("*")]
    #[endpoint(removeLiquidity)]
    fn ash_remove_liquidity_stable(&self, token_amount_min: MultiValueEncoded<BigUint>);

    // ═══════════════════════════════════════════════════════════════════════════
    // AshSwap V2 (CurveCrypto)
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(exchange)]
    fn ash_exchange_crypto(&self, min_dy: BigUint);

    #[payable("*")]
    #[endpoint(addLiquidity)]
    fn ash_add_liquidity_crypto(
        &self,
        min_mint_amount: BigUint,
        opt_receiver: OptionalValue<ManagedAddress>,
    );

    #[payable("*")]
    #[endpoint(removeLiquidity)]
    fn ash_remove_liquidity_crypto(
        &self,
        min_amounts: ManagedVec<BigUint>,
        opt_receiver: OptionalValue<ManagedAddress>,
    );

    // ═══════════════════════════════════════════════════════════════════════════
    // Liquid Staking (Xoxno, Hatom)
    // ═══════════════════════════════════════════════════════════════════════════

    /// Xoxno liquid staking delegate (xEGLD, LXOXNO)
    #[payable("*")]
    #[endpoint(delegate)]
    fn delegate(&self, to: OptionalValue<ManagedAddress>);

    /// Hatom liquid staking delegate (sEGLD)
    #[payable("*")]
    #[endpoint(delegate)]
    fn delegate_hatom(&self);

    // ═══════════════════════════════════════════════════════════════════════════
    // Hatom Lending
    // ═══════════════════════════════════════════════════════════════════════════

    #[payable("*")]
    #[endpoint(mint)]
    fn hatom_mint(&self);

    #[payable("*")]
    #[endpoint(redeem)]
    fn hatom_redeem(&self, underlying_amount: OptionalValue<BigUint>);
}
