use std::{collections::HashMap, env, fs, sync::Arc};

use serde::Serialize;
use serde_json::json;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig},
};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    pubkey,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_instruction,
    transaction::{self, Transaction},
};

use crate::{
    filter::FilterSet,
    pump_interaction::{
        constans::{self, programs},
        instructions::{Buy, buy, create_account},
        wrappers::TokenAccounts,
    },
};

use base64::{Engine as _, engine::general_purpose};
use reqwest::Client;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub struct CloneableKeypair(pub Keypair);

impl Clone for CloneableKeypair {
    fn clone(&self) -> Self {
        let bytes = self.0.to_bytes();
        let new_keypair =
            Keypair::from_bytes(&bytes).expect("Failed to recover keypair from bytes");

        CloneableKeypair(new_keypair)
    }
}

impl Deref for CloneableKeypair {
    type Target = Keypair;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for CloneableKeypair {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<Keypair> for CloneableKeypair {
    fn from(kp: Keypair) -> Self {
        CloneableKeypair(kp)
    }
}

#[derive(Clone)]
pub struct AutoBuyConfig {
    pub wallet: CloneableKeypair,
    pub params: Params,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Params {
    pub lamport_amount: u64,
    pub priority_fee: u64,
    pub slippage: f32,
    pub bribe: u64,
    pub filters: FilterSet,
    #[serde(default)]
    pub use_leader_send: bool,
}

pub struct BuyAutomata {
    pub enabled: bool,
    client: Arc<RpcClient>,

    pub config: AutoBuyConfig,
    pub active_twitter: bool,
    pub active_migrate: bool,
    pub active_whitelist: bool,
}

impl BuyAutomata {
    pub fn with_config(client: Arc<RpcClient>, config: AutoBuyConfig) -> Self {
        Self {
            enabled: false,
            client,
            config,
            active_twitter: false,
            active_migrate: false,
            active_whitelist: false,
        }
    }

    pub async fn buy(&self, token: &tokenir_ui::Token) -> Result<(), Error> {
        let wallet = &self.config.wallet;
        let accounts = TokenAccounts::new(&token.mint, &token.dev, token.token_2022);

        let token_program = match token.token_2022 {
            true => constans::programs::TOKEN_PROGRAM_2022,
            false => constans::programs::TOKEN_PROGRAM,
        };

        let ata_ix = create_associated_token_account_idempotent(
            &wallet.pubkey(),
            &wallet.pubkey(),
            &token.mint,
            &token_program,
        );

        const COMPUTE_LIMIT: u32 = 120_000;

        let compute_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_LIMIT);
        let micro_price = ((self.config.params.priority_fee as u128) * 1_000_000u128
            / (COMPUTE_LIMIT as u128)) as u64;

        let priority_fee_ix = ComputeBudgetInstruction::set_compute_unit_price(micro_price);

        let accounts_clone = accounts.clone();

        let mut bonded = accounts.bond();
        let curve = bonded.update(&self.client).await;

        let Some(curve) = curve else {
            println!("Not found!");
            return Err(Error::BoundingCurveNotFound);
        };

        let buy = buy(
            &wallet,
            &accounts_clone,
            &Buy::new(
                curve.buy(self.config.params.lamport_amount).unwrap(),
                self.config.params.lamport_amount
                    + (self.config.params.lamport_amount as f32 * self.config.params.slippage)
                        as u64,
            ),
            token.token_2022,
        );

        let tip = system_instruction::transfer(
            &self.config.wallet.pubkey(),
            &pubkey!("D1Mc6j9xQWgR1o1Z7yU5nVVXFQiAYx7FG9AW1aVfwrUM"),
            self.config.params.bribe,
        );

        let tx = self
            .proccess_transaction(&[compute_limit_ix, priority_fee_ix, ata_ix, buy, tip])
            .await?;

        let _ = self.send_via_jito(&tx).await;

        Ok(())
    }

    async fn proccess_transaction(
        &self,
        instructions: &[Instruction],
    ) -> Result<Transaction, Error> {
        let Ok(blockhash) = self.client.get_latest_blockhash().await else {
            return Err(Error::BlockHashFetchFailed);
        };

        let tx = Transaction::new_signed_with_payer(
            instructions,
            Some(&self.config.wallet.pubkey()),
            &[self.config.wallet.insecure_clone()],
            blockhash,
        );

        Ok(tx)
    }

    async fn send_via_jito(&self, tx: &Transaction) -> Result<(), Error> {
        let serialized = bincode::serialize(tx).map_err(|_| Error::TransactionError)?;
        let encoded = general_purpose::STANDARD.encode(&serialized);

        let body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "sendTransaction",
            "params": [
                encoded,
                {
                    "encoding": "base64",
                    "skipPreflight": true
                }
            ]
        })
        .to_string();

        let client = Client::new();

        let resp = client
            .post("http://fra-sender.helius-rpc.com/fast")
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| Error::TransactionError)?;

        let text = resp.text().await.map_err(|_| Error::TransactionError)?;
        println!("Jito response: {}", text);

        Ok(())
    }
}

#[derive(Debug)]
pub enum Error {
    BlockHashFetchFailed,
    TransactionError,
    BoundingCurveNotFound,
    LeaderScheduleFetchFailed,
    SlotFetchFailed,
    LeaderNotFound,
}

impl AutoBuyConfig {
    pub fn load() -> AutoBuyConfig {
        match fs::read_to_string("./config.json") {
            Ok(data) => {
                let params: Params = serde_json::from_str(&data).unwrap_or(Params {
                    lamport_amount: 0,
                    priority_fee: 0,
                    slippage: 0.03,
                    bribe: 100_000,
                    filters: FilterSet::new(),
                    use_leader_send: false,
                });

                let config = AutoBuyConfig {
                wallet: Keypair::from_base58_string(&env::var("WALLET").unwrap_or("2V6BtXLQzqAEsjNgww1a7Z4nCeV25xYxngG4jRGSYuch4PXqLed4VPTAcLxLtgUgFF7tRXkMHfdZE9MB4P3SGWRf".to_string())).into(),
                params,
            };

                let _ = config.to_file();
                config
            }

            Err(_) => {
                let blacklist = AutoBuyConfig {
                wallet: Keypair::from_base58_string(&env::var("WALLET").unwrap_or("2V6BtXLQzqAEsjNgww1a7Z4nCeV25xYxngG4jRGSYuch4PXqLed4VPTAcLxLtgUgFF7tRXkMHfdZE9MB4P3SGWRf".to_string())).into(),
                params: Params {
                    lamport_amount: 100,
                    priority_fee: 0,
                    slippage: 0.5,
                    bribe: 100_000,
                    filters: FilterSet::new(),
                    use_leader_send: false,
                },
            };
                let _ = blacklist.to_file();
                blacklist
            }
        }
    }

    pub fn to_file(&self) -> Result<(), std::io::Error> {
        let _ = fs::write(
            "./config.json",
            serde_json::to_string(&self.params).unwrap(),
        )?;
        Ok(())
    }
}
