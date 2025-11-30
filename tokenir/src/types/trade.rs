use crate::logs::{BuyEvent, SellEvent};

#[derive(Clone)]
pub enum Trade {
    Buy(BuyEvent),
    Sell(SellEvent),
}

impl Trade {
    pub fn mcap(&self) -> u64 {
        match self {
            Self::Buy(data) => data.virtual_sol_reserves_before,
            Self::Sell(data) => data.virtual_sol_reserves_before,
        }
    }

    pub fn mcap_after(&self) -> u64 {
        match self {
            Self::Buy(data) => data.virtual_sol_reserves_after,
            Self::Sell(data) => data.virtual_sol_reserves_after,
        }
    }

    pub fn reserves(&self) -> u64 {
        match self {
            Self::Buy(data) => data.virtual_token_reserves,
            Self::Sell(data) => data.virtual_token_reserves,
        }
    }
}
