multiversx_sc::imports!();

#[multiversx_sc::module]
pub trait Events {
    /// Final settlement: original inputs, paid outputs, and fees credited by this call.
    /// Admin fees include retained dust, merged by token. Empty lists mean no fees.
    /// Referral ID is the active ID accepted for fee calculation, or 0 if none.
    /// Each payment list is one encoded topic containing token, nonce, and amount.
    #[event("aggregate")]
    fn aggregate_event(
        &self,
        #[indexed] caller: &ManagedAddress,
        #[indexed] referral_id: u64,
        #[indexed] inputs: &PaymentVec<Self::Api>,
        #[indexed] outputs: &PaymentVec<Self::Api>,
        #[indexed] admin_fees: &PaymentVec<Self::Api>,
        #[indexed] referral_fees: &PaymentVec<Self::Api>,
    );
}
