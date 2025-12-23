use crate::constants::{HATOM_CONTROLLER, XEXCHANGE_ROUTER};
use crate::types::PairTokens;

multiversx_sc::imports!();

#[multiversx_sc::module]
pub trait Storage {
    #[storage_mapper_from_address("pair_map")]
    fn pair_map(
        &self,
        address: ManagedAddress,
    ) -> MapMapper<PairTokens<Self::Api>, ManagedAddress, ManagedAddress>;

    /// Stores a whitelisted market address given a token identifier.
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
}
