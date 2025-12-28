use borsh::{BorshDeserialize, io};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

#[derive(Debug)]
pub enum Event {
    Create(CreateEvent),
    Buy(BuyEvent),
    Sell(SellEvent),
}

impl Event {
    pub fn mint(&self) -> &Pubkey {
        match self {
            Event::Create(create_event) => &create_event.mint,
            Event::Buy(buy_event) => &buy_event.mint,
            Event::Sell(sell_event) => &sell_event.mint,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreateEvent {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub user: Pubkey,
    pub token_2022: bool,
}

impl BorshDeserialize for CreateEvent {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let name = String::deserialize_reader(reader)?;
        let symbol = String::deserialize_reader(reader)?;
        let uri = String::deserialize_reader(reader)?;
        let mint = Pubkey::deserialize_reader(reader)?;
        let bonding_curve = Pubkey::deserialize_reader(reader)?;
        let user = Pubkey::deserialize_reader(reader)?;

        let token_2022 = match bool::deserialize_reader(reader) {
            Ok(v) => v,
            Err(_) => false,
        };

        Ok(Self {
            name,
            symbol,
            uri,
            mint,
            bonding_curve,
            user,
            token_2022,
        })
    }
}

#[derive(Clone, Debug, BorshDeserialize)]
pub struct CreateEventV2 {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub user: Pubkey,
    pub creator: Pubkey,
    pub timestamp: i64,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub token_total_supply: u64,
    pub token_program: Pubkey,
    pub is_mayhem_mode: bool,
}

impl From<CreateEventV2> for CreateEvent {
    fn from(v: CreateEventV2) -> Self {
        Self {
            name: v.name,
            symbol: v.symbol,
            uri: v.uri,
            mint: v.mint,
            bonding_curve: v.bonding_curve,
            user: v.user,
            token_2022: v.token_program == pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"),
        }
    }
}

#[derive(Clone, Debug, BorshDeserialize)]
pub struct BuyEvent {
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub user: Pubkey,
    pub timestamp: i64,
    pub virtual_sol_reserves_before: u64,
    pub virtual_sol_reserves_after: u64,
    pub virtual_token_reserves: u64,
}

#[derive(Clone, Debug, BorshDeserialize)]
pub struct SellEvent {
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub user: Pubkey,
    pub timestamp: i64,
    pub virtual_sol_reserves_before: u64,
    pub virtual_sol_reserves_after: u64,
    pub virtual_token_reserves: u64,
}

#[derive(Clone, Debug, BorshDeserialize)]
pub struct TradeEvent {
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub is_buy: bool,
    pub user: Pubkey,
    pub timestamp: i64,
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
}

#[derive(Clone, Debug, BorshDeserialize)]
pub struct BuyEventAMM {
    pub timestamp: i64,
    pub base_amount_out: u64,
    pub max_quote_amount_in: u64,
    pub user_base_token_reserves: u64,
    pub user_quote_token_reserves: u64,
    pub pool_base_token_reserves: u64,
    pub pool_quote_token_reserves: u64,
    pub quote_amount_in: u64,
    pub lp_fee_basis_points: u64,
    pub lp_fee: u64,
    pub protocol_fee_basis_points: u64,
    pub protocol_fee: u64,
    pub quote_amount_in_with_lp_fee: u64,
    pub user_quote_amount_in: u64,
    pub pool: Pubkey,
    pub user: Pubkey,
    pub user_base_token_account: Pubkey,
    pub user_quote_token_account: Pubkey,
    pub protocol_fee_recipient: Pubkey,
    pub protocol_fee_recipient_token_account: Pubkey,
}

#[derive(Clone, Debug, BorshDeserialize)]
pub struct SellEventAMM {
    pub timestamp: i64,
    pub base_amount_in: u64,
    pub min_quote_amount_out: u64,
    pub user_base_token_reserves: u64,
    pub user_quote_token_reserves: u64,
    pub pool_base_token_reserves: u64,
    pub pool_quote_token_reserves: u64,
    pub quote_amount_out: u64,
    pub lp_fee_basis_points: u64,
    pub lp_fee: u64,
    pub protocol_fee_basis_points: u64,
    pub protocol_fee: u64,
    pub quote_amount_out_without_lp_fee: u64,
    pub user_quote_amount_out: u64,
    pub pool: Pubkey,
    pub user: Pubkey,
    pub user_base_token_account: Pubkey,
    pub user_quote_token_account: Pubkey,
    pub protocol_fee_recipient: Pubkey,
    pub protocol_fee_recipient_token_account: Pubkey,
}

impl From<BuyEventAMM> for BuyEvent {
    fn from(e: BuyEventAMM) -> Self {
        //if base is more, spl is base
        if e.pool_base_token_reserves > e.pool_quote_token_reserves {
            return BuyEvent {
                mint: e.pool,
                sol_amount: e.base_amount_out,
                token_amount: e.quote_amount_in,
                user: e.user,
                timestamp: e.timestamp,
                virtual_sol_reserves_before: e.pool_quote_token_reserves,
                virtual_sol_reserves_after: e.pool_quote_token_reserves,
                virtual_token_reserves: e.pool_base_token_reserves,
            };
        }
        //in other case, sol is base
        BuyEvent {
            mint: e.pool,
            sol_amount: e.base_amount_out,
            token_amount: e.quote_amount_in,
            user: e.user,
            timestamp: e.timestamp,
            virtual_sol_reserves_before: e.pool_base_token_reserves,
            virtual_sol_reserves_after: e.pool_base_token_reserves,
            virtual_token_reserves: e.pool_quote_token_reserves,
        }
    }
}

impl From<SellEventAMM> for SellEvent {
    fn from(e: SellEventAMM) -> Self {
        SellEvent {
            mint: e.pool, // см. выше, проверь логику
            sol_amount: e.base_amount_in,
            token_amount: e.quote_amount_out,
            user: e.user,
            timestamp: e.timestamp,
            virtual_sol_reserves_before: e.pool_base_token_reserves,
            virtual_sol_reserves_after: e.pool_base_token_reserves,
            virtual_token_reserves: e.pool_quote_token_reserves,
        }
    }
}
