use borsh::{io, BorshDeserialize};
use solana_sdk::pubkey::Pubkey;
use std::time::{SystemTime, UNIX_EPOCH};

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

// Helper function for current timestamp in seconds
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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
    pub timestamp: i64,
}

impl TryFrom<PumpCreateEvent> for CreateEvent {
    type Error = solana_sdk::pubkey::ParsePubkeyError;

    fn try_from(e: PumpCreateEvent) -> Result<Self, Self::Error> {
        Ok(Self {
            name: e.name,
            symbol: e.symbol,
            uri: e.uri,

            // PumpPortal sends mint as base58 string
            mint: Pubkey::from_str_const(&e.mint),

            // PumpPortal does NOT expose bonding curve directly
            // Pool is the best proxy
            bonding_curve: Pubkey::from_str_const(&e.pool),

            // PumpPortal does not expose creator/user reliably
            // Use mint authority / zero fallback
            user: Pubkey::default(),

            // Pump.fun tokens are Token-2022 by default
            token_2022: true,

            // PumpPortal does not send timestamp → set locally
            timestamp: current_timestamp(),
        })
    }
}

mod pubkey_from_string {
    use serde::{Deserialize, Deserializer};
    use solana_sdk::pubkey::Pubkey;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Pubkey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Pubkey>().map_err(serde::de::Error::custom)
    }
}

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PumpCreateEvent {
    pub signature: String,
    pub mint: String,
    #[serde(rename = "traderPublicKey")]
    pub trader_public_key: String,
    #[serde(rename = "txType")]
    pub tx_type: String,
    pub name: String,
    pub symbol: String,
    pub uri: String,

    // Initial buy stats
    #[serde(rename = "solAmount")]
    pub sol_amount: f64,
    #[serde(rename = "initialBuy")]
    pub initial_buy: f64,
    #[serde(rename = "marketCapSol")]
    pub market_cap_sol: f64,

    // Bonding curve details
    #[serde(rename = "bondingCurveKey")]
    pub bonding_curve_key: String,
    #[serde(rename = "vTokensInBondingCurve")]
    pub v_tokens_in_bonding_curve: f64,
    #[serde(rename = "vSolInBondingCurve")]
    pub v_sol_in_bonding_curve: f64,

    // Extra flags
    pub is_mayhem_mode: bool,
    pub pool: String,
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

        // Set timestamp to current time if not in payload
        let timestamp = current_timestamp();

        Ok(Self {
            name,
            symbol,
            uri,
            mint,
            bonding_curve,
            user,
            token_2022,
            timestamp,
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
            token_2022: v.token_program
                == Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"),
            timestamp: v.timestamp, // carry over timestamp from V2
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

// AMM events
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

// Conversions from AMM
impl From<BuyEventAMM> for BuyEvent {
    fn from(e: BuyEventAMM) -> Self {
        if e.pool_base_token_reserves > e.pool_quote_token_reserves {
            BuyEvent {
                mint: e.pool,
                sol_amount: e.base_amount_out,
                token_amount: e.quote_amount_in,
                user: e.user,
                timestamp: e.timestamp,
                virtual_sol_reserves_before: e.pool_quote_token_reserves,
                virtual_sol_reserves_after: e.pool_quote_token_reserves,
                virtual_token_reserves: e.pool_base_token_reserves,
            }
        } else {
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
}

impl From<SellEventAMM> for SellEvent {
    fn from(e: SellEventAMM) -> Self {
        SellEvent {
            mint: e.pool,
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
