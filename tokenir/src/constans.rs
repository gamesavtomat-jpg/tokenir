pub mod requests {
    use crate::requests::CreatorHistory;
    use serde::Deserialize;
    use solana_sdk::pubkey::Pubkey;

    pub const SUBSCRIBE_REQUEST_PUMP: &'static str = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "logsSubscribe",
        "params": [
            {
                "mentions": ["6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"]
            },
            {
                "commitment": "confirmed"
            }
        ]
    }"#;

    pub const SUBSCRIBE_REQUEST_AMM: &'static str = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "logsSubscribe",
        "params": [
            {
                "mentions": ["pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"]
            },
            {
                "commitment": "confirmed"
            }
        ]
    }"#;

    #[derive(Debug)]
    pub enum HistoryError {
        RequestError(reqwest::Error),
        DeserializationError(serde_json::Error),
    }

    impl From<reqwest::Error> for HistoryError {
        fn from(value: reqwest::Error) -> Self {
            Self::RequestError(value)
        }
    }

    impl From<serde_json::Error> for HistoryError {
        fn from(value: serde_json::Error) -> Self {
            Self::DeserializationError(value)
        }
    }

    use reqwest::{ClientBuilder, Url, cookie::Jar};

    pub async fn get_user_created_coins(user: &Pubkey) -> Result<CreatorHistory, HistoryError> {
        let request = format!("https://api.axiom.trade/dev-tokens-v2?devAddress={}", user);

        let jar = Jar::default();
        jar.add_cookie_str(
            &std::env::var("COOKIE").expect("No COOKIE in .env"),
            &"https://api.axiom.trade".parse::<Url>().unwrap(),
        );

        let client = ClientBuilder::new()
            .cookie_store(true)
            .cookie_provider(jar.into())
            .build()
            .unwrap();

        let response = client.get(request).send().await?;

        let body = response.text().await?;

        let history: CreatorHistory = serde_json::from_str(&body)?;
        Ok(history)
    }
}

pub mod helper {
    use std::env;

    use reqwest::Client;
    use serde::Deserialize;
    use serde::Serialize;
    use solana_sdk::pubkey;
    use solana_sdk::pubkey::Pubkey;

    use crate::requests::Metadata;

    pub fn pool_pda(base_mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                b"pool",
                &0u16.to_le_bytes(),
                pool_authority(base_mint).0.as_ref(),
                base_mint.as_ref(),
                pubkey!("So11111111111111111111111111111111111111112").as_ref(),
            ],
            &pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"),
        )
    }

    pub fn pool_authority(mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                &[
                    112, 111, 111, 108, 45, 97, 117, 116, 104, 111, 114, 105, 116, 121,
                ],
                mint.as_ref(),
            ],
            &pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        )
    }

    pub struct PriceImpact {
        pub price_before: f64,
        pub price_after: f64,
        pub impact_pct: f64,
        pub mcap_before: u64, // в лампортах
        pub mcap_after: u64,  // в лампортах
    }

    pub const METAPLEX_PROGRAM: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

    pub fn metadata(mint: &Pubkey) -> (Pubkey, u8) {
        let seeds = &[b"metadata", METAPLEX_PROGRAM.as_ref(), mint.as_ref()];
        Pubkey::find_program_address(seeds, &METAPLEX_PROGRAM)
    }

    pub fn calc_price_impact(
        virtual_sol_reserves: u64,
        virtual_token_reserves: u64,
        sol_amount: u64,
        token_amount: u64,
        is_buy: bool,
        total_supply: u64,
    ) -> PriceImpact {
        let v_sol = virtual_sol_reserves as f64;
        let v_token = virtual_token_reserves as f64;

        // цена до (в SOL)
        let price_before = v_sol / v_token;

        // обновляем резервы
        let (new_sol, new_token) = if is_buy {
            (v_sol + sol_amount as f64, v_token - token_amount as f64)
        } else {
            (v_sol - sol_amount as f64, v_token + token_amount as f64)
        };

        // цена после (в SOL)
        let price_after = new_sol / new_token;

        // % импакта
        let impact_pct = (price_after - price_before) / price_before * 100.0;

        // market cap в лампортах
        let mcap_before = (price_before * 1_000_000.0 * total_supply as f64) as u64;
        let mcap_after = (price_after * 1_000_000.0 * total_supply as f64) as u64;

        PriceImpact {
            price_before,
            price_after,
            impact_pct,
            mcap_before,
            mcap_after,
        }
    }

    #[derive(Debug)]
    pub enum Error {
        Reqwest(reqwest::Error),
        SerdeJson(serde_json::Error),
        SomeFuckedUpShit,
    }

    impl From<reqwest::Error> for Error {
        fn from(e: reqwest::Error) -> Self {
            Error::Reqwest(e)
        }
    }

    impl From<serde_json::Error> for Error {
        fn from(e: serde_json::Error) -> Self {
            Error::SerdeJson(e)
        }
    }

    #[derive(Debug, Deserialize)]
    struct ApiResponseTokenMetadata {
        data: ApiData,
    }

    #[derive(Debug, Deserialize)]
    struct ApiData {
        name: String,
        symbol: String,
        description: Option<String>,
        logo: Option<String>,
        socials: Option<Socials>,
    }

    #[derive(Debug, Deserialize)]
    struct Socials {
        twitter: Option<String>,
        website: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct MoralisMetaplex {
        #[serde(rename = "metadataUri")]
        pub metadata_uri: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct MoralisResponse {
        pub metaplex: Option<MoralisMetaplex>,
    }

    pub async fn get_uri(mint: &Pubkey) -> Result<MoralisMetaplex, Error> {
        let api_key = env::var("API_KEY_MORALIS").map_err(|_| Error::SomeFuckedUpShit)?;

        let url = format!(
            "https://solana-gateway.moralis.io/token/mainnet/{}/metadata",
            mint
        );

        let client = Client::new();

        let raw = client
            .get(&url)
            .header("X-API-Key", api_key)
            .header("accept", "application/json")
            .header("User-Agent", "reqwest") // <-- curl includes UA by default
            .send()
            .await?
            .text()
            .await?;

        println!("Moralis raw: {}", raw);

        let response: MoralisResponse = serde_json::from_str(&raw)?;

        response.metaplex.ok_or(Error::SomeFuckedUpShit)
    }

    pub async fn get_metadata(url: &str) -> Result<Metadata, Error> {
        let response = reqwest::get(url).await?;
        let body = response.text().await?;

        let metadata: Metadata = serde_json::from_str(&body)?;
        Ok(metadata)
    }

    #[derive(Deserialize, Debug)]
    struct ApiResponse {
        community_info: CommunityInfo,
        status: String,
        msg: Option<String>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct CommunityInfo {
        pub id: String,
        pub name: String,
        pub description: Option<String>,
        pub member_count: Option<u64>,
        pub moderator_count: Option<u64>,
        pub created_at: Option<String>,
        pub creator: Creator,
    }

    impl CommunityInfo {
        pub fn creator(&self) -> &Creator {
            &self.creator
        }
    }

    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct Creator {
        pub id: String,
        pub screen_name: Option<String>,
    }

    impl Creator {
        pub fn screen_name(&self) -> &Option<String> {
            &self.screen_name
        }
    }

    pub const PUMP_FUN: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

    pub fn bounding_curve(mint: &Pubkey) -> (Pubkey, u8) {
        let seeds = &[b"bonding-curve", mint.as_ref()];
        Pubkey::find_program_address(seeds, &PUMP_FUN)
    }

    pub async fn get_community_by_id(
        api_key: &str,
        community_id: &str,
    ) -> Result<CommunityInfo, Error> {
        let client = Client::new();
        let resp = client
            .get(format!(
                "https://api.twitterapi.io/twitter/community/info?community_id={}",
                community_id
            ))
            .header("X-API-Key", api_key)
            .header("User-Agent", "reqwest")
            .send()
            .await?
            .error_for_status()?;

        let text = resp.text().await?;

        let com: ApiResponse = serde_json::from_str(&text)?;
        Ok(com.community_info)
    }

    pub fn parse_community_id(url: &str) -> Option<String> {
        if url.contains("/i/communities/") {
            url.trim_end_matches('/')
                .split('/')
                .last()
                .map(|id| id.to_string())
        } else {
            None
        }
    }

    use crate::requests::PriceResponse;

    pub async fn fetch_solana_price() -> Result<f64, reqwest::Error> {
        let url = "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd";
        let resp: PriceResponse = reqwest::get(url).await?.json().await?;
        Ok(resp.solana.usd)
    }
}
