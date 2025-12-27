multiversx_sc::imports!();

use crate::constants::TOTAL_FEE;
use crate::errors::{
    ERR_FEE_EXCEEDS_100, ERR_NOT_REFERRAL_OWNER, ERR_REFERRAL_FEE_EXCEEDS_50,
    ERR_REFERRAL_NOT_FOUND,
};
use crate::types;

/// Admin configuration module for referral and fee management
#[multiversx_sc::module]
pub trait Config: crate::storage::Storage {
    // --- Admin Endpoints ---

    /// Add a new referral with the given owner and fee
    /// Returns the new referral ID
    /// Note: Referral fee is capped at 50% because total fees = referral_fee + admin_fee (matching)
    #[only_owner]
    #[endpoint(addReferral)]
    fn add_referral(&self, owner: ManagedAddress, fee: u32) -> u64 {
        require!(fee <= TOTAL_FEE / 2, ERR_REFERRAL_FEE_EXCEEDS_50);
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
    /// Note: Referral fee is capped at 50% because total fees = referral_fee + admin_fee (matching)
    #[only_owner]
    #[endpoint(setReferralFee)]
    fn set_referral_fee(&self, id: u64, fee: u32) {
        require!(!self.referral_config(id).is_empty(), ERR_REFERRAL_NOT_FOUND);
        require!(fee <= TOTAL_FEE / 2, ERR_REFERRAL_FEE_EXCEEDS_50);
        self.referral_config(id).update(|c| c.fee = fee);
    }

    /// Enable or disable a referral
    #[only_owner]
    #[endpoint(setReferralActive)]
    fn set_referral_active(&self, id: u64, active: bool) {
        require!(!self.referral_config(id).is_empty(), ERR_REFERRAL_NOT_FOUND);
        self.referral_config(id).update(|c| c.active = active);
    }

    /// Change the owner of an existing referral
    #[only_owner]
    #[endpoint(setReferralOwner)]
    fn set_referral_owner(&self, id: u64, new_owner: ManagedAddress) {
        require!(!self.referral_config(id).is_empty(), ERR_REFERRAL_NOT_FOUND);
        self.referral_config(id).update(|c| c.owner = new_owner);
    }

    /// Set the static fee for trades without a referral
    #[only_owner]
    #[endpoint(setStaticFee)]
    fn set_static_fee(&self, fee: u32) {
        require!(fee <= TOTAL_FEE, ERR_FEE_EXCEEDS_100);
        self.static_fee().set(fee);
    }

    // --- Claim Endpoints ---

    /// Claim accumulated referral fees for a given referral ID
    /// Can only be called by the referral owner
    #[endpoint(claimReferralFees)]
    fn claim_referral_fees(&self, referral_id: u64) {
        require!(
            !self.referral_config(referral_id).is_empty(),
            ERR_REFERRAL_NOT_FOUND
        );
        let config = self.referral_config(referral_id).get();
        let caller = self.blockchain().get_caller();
        require!(caller == config.owner, ERR_NOT_REFERRAL_OWNER);

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
