use std::{env, fs, sync::Arc, collections::HashMap};

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
    signature::Keypair,
    signer::Signer,
    system_instruction,
    transaction::{self, Transaction},
    pubkey::Pubkey,
};

use crate::{
    filter::FilterSet,
    pump_interaction::{
        constans::{self, programs},
        instructions::{Buy, buy, create_account},
        wrappers::TokenAccounts,
    },
};

use std::ops::{Deref, DerefMut};
use base64::{Engine as _, engine::general_purpose};
use reqwest::Client;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;

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
    pub enabled : bool,
    client: Arc<RpcClient>,
    leader_cache: tokio::sync::RwLock<LeaderCache>,

    pub config: AutoBuyConfig,
    pub active_twitter: bool,
    pub active_migrate: bool,
    pub active_whitelist: bool,
}

struct LeaderCache {
    schedule: Option<HashMap<String, Vec<usize>>>,
    validator_rpcs: HashMap<String, String>,
    last_update: std::time::Instant,
}

impl LeaderCache {
    fn new() -> Self {
        Self {
            schedule: None,
            validator_rpcs: Self::load_known_validators(),
            last_update: std::time::Instant::now(),
        }
    }

    fn load_known_validators() -> HashMap<String, String> {
        // Known validators with public RPC endpoints
        // You should expand this list or fetch dynamically
        HashMap::from([
            ("7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2".to_string(), 
             "https://api.mainnet-beta.solana.com".to_string()),
            ("GE6atKoWiQ2pt3zL7N13pjNHjdLVys8LinG8qeJLcAiL".to_string(), 
             "https://api.mainnet-beta.solana.com".to_string()),
            // Add more known validators here
        ])
    }

    fn needs_refresh(&self) -> bool {
        self.schedule.is_none() || self.last_update.elapsed().as_secs() > 60
    }
}

impl BuyAutomata {
    pub fn with_config(client: Arc<RpcClient>, config: AutoBuyConfig) -> Self {
        Self {
            enabled : false,
            client,
            leader_cache: tokio::sync::RwLock::new(LeaderCache::new()),
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
            token.token_2022
        );

        let tip = system_instruction::transfer(
            &self.config.wallet.pubkey(),
            &pubkey!("ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt"),
            self.config.params.bribe,
        );

        let tx = self
            .proccess_transaction(&[compute_limit_ix, priority_fee_ix, ata_ix, buy, tip])
            .await?;

        // Choose submission method
        if self.config.params.use_leader_send {
            println!("Attempting direct leader send...");
            match self.send_to_leader(&tx).await {
                Ok(_) => println!("Sent directly to leader!"),
                Err(e) => {
                    println!("Leader send failed, falling back to Jito: {:?}", e);
                    let _ = self.send_via_jito(&tx).await;
                }
            }
        } else {
            let _ = self.send_via_jito(&tx).await;
        }

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

    async fn send_to_leader(&self, tx: &Transaction) -> Result<(), Error> {
        // Update leader cache if needed
        {
            let cache = self.leader_cache.read().await;
            if cache.needs_refresh() {
                drop(cache);
                self.refresh_leader_info().await?;
            }
        }

        // Get current leader
        let (leader_pubkey, leader_rpc) = self.get_current_leader().await?;
        println!("Current leader: {}", leader_pubkey);
        println!("Leader RPC: {}", leader_rpc);

        // Create client for leader
        let leader_client = RpcClient::new_with_commitment(
            leader_rpc.clone(),
            CommitmentConfig::confirmed(),
        );

        // Send transaction with skip_preflight
        let config = RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(solana_sdk::commitment_config::CommitmentLevel::Processed),
            encoding: None,
            max_retries: Some(0),
            min_context_slot: None,
        };

        let signature = leader_client
            .send_transaction_with_config(tx, config)
            .await
            .map_err(|_| Error::TransactionError)?;

        println!("Transaction sent to leader: {}", signature);

        Ok(())
    }

    async fn refresh_leader_info(&self) -> Result<(), Error> {
        println!("Refreshing leader schedule...");
        
        let schedule = self
            .client
            .get_leader_schedule(None)
            .await
            .map_err(|_| Error::LeaderScheduleFetchFailed)?;

        // Try to update validator RPC endpoints from cluster nodes
        let mut validator_rpcs = HashMap::new();
        if let Ok(nodes) = self.client.get_cluster_nodes().await {
            for node in nodes {
                if let Some(rpc) = node.rpc {
                    let rpc_url = format!("http://{}:{}", rpc.ip(), rpc.port());
                    validator_rpcs.insert(node.pubkey, rpc_url);
                }
            }
            println!("Found {} validator RPC endpoints", validator_rpcs.len());
        }

        let mut cache = self.leader_cache.write().await;
        cache.schedule =schedule;
        
        // Merge with known validators
        if !validator_rpcs.is_empty() {
            cache.validator_rpcs.extend(validator_rpcs);
        }
        
        cache.last_update = std::time::Instant::now();

        Ok(())
    }

    async fn get_current_leader(&self) -> Result<(String, String), Error> {
        let cache = self.leader_cache.read().await;
        
        let schedule = cache
            .schedule
            .as_ref()
            .ok_or(Error::LeaderScheduleFetchFailed)?;

        let current_slot = self
            .client
            .get_slot()
            .await
            .map_err(|_| Error::SlotFetchFailed)?;

        // Find current leader
        let leader_pubkey = Self::find_leader_at_slot(schedule, current_slot)?;

        // Get leader's RPC endpoint
        let leader_rpc = cache
            .validator_rpcs
            .get(&leader_pubkey)
            .cloned()
            .unwrap_or_else(|| {
                // Fallback to main RPC if we don't have the leader's endpoint
                println!("Warning: Leader RPC not found, using main RPC");
                "https://api.mainnet-beta.solana.com".to_string()
            });

        Ok((leader_pubkey, leader_rpc))
    }

    fn find_leader_at_slot(
        schedule: &HashMap<String, Vec<usize>>,
        slot: u64,
    ) -> Result<String, Error> {
        // Each epoch has multiple slots, leaders rotate every 4 slots
        let total_slots: usize = schedule.values().map(|v| v.len()).sum();
        let slot_index = (slot as usize) % total_slots;

        for (validator, slots) in schedule {
            if slots.contains(&slot_index) {
                return Ok(validator.clone());
            }
        }

        Err(Error::LeaderNotFound)
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
                { "encoding": "base64" }
            ]
        })
        .to_string();

        let client = Client::new();

        let resp = client
            .post("https://mainnet.block-engine.jito.wtf/api/v1/transactions")
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