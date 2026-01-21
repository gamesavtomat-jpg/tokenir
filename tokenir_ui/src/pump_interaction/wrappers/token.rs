use crate::{
    pump_interaction::accounts::BondingCurve,
    pump_interaction::constans::deriving::{
        associated_token_address, bounding_curve, creator_vault, metadata,
    },
};
use borsh::BorshDeserialize;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;

#[derive(Clone)]
pub struct TokenAccounts {
    mint: Pubkey,
    bonding_curve: Pubkey,
    associated_bonding_curve: Pubkey,
    metadata: Pubkey,
    creator: Pubkey,
    creator_vault: Pubkey,
}

impl TokenAccounts {
    pub fn new(mint: &Pubkey, creator: &Pubkey, token_2022: bool) -> Self {
        let (bonding_curve, _) = bounding_curve(mint);
        let (associated_bonding_curve, _) =
            associated_token_address(&bonding_curve, mint, token_2022);
        let (metadata, _) = metadata(&mint);

        let (creator_vault, _) = creator_vault(creator);

        Self {
            mint: *mint,
            bonding_curve,
            associated_bonding_curve,
            creator: *creator,
            metadata,
            creator_vault,
        }
    }

    pub fn bond(self) -> Token {
        Token::new(self)
    }

    pub fn mint(&self) -> &Pubkey {
        &self.mint
    }

    pub fn bonding_curve(&self) -> &Pubkey {
        &self.bonding_curve
    }

    pub fn associated_bonding_curve(&self) -> &Pubkey {
        &self.associated_bonding_curve
    }

    pub fn creator(&self) -> &Pubkey {
        &self.creator
    }

    pub fn metadata(&self) -> &Pubkey {
        &self.metadata
    }

    pub fn creator_vault(&self) -> &Pubkey {
        &self.creator_vault
    }
}

pub struct Token {
    accounts: TokenAccounts,
    bonding_curve: BondingCurve,
}

impl Token {
    pub fn new(accounts: TokenAccounts) -> Self {
        Self {
            accounts,
            bonding_curve: BondingCurve::default(),
        }
    }

    pub async fn update(&mut self, client: &RpcClient) -> Option<&BondingCurve> {
        let Some(bonding_curve) = self.fetch_bounding_curve_data(client).await else {
            return None;
        };

        self.bonding_curve = bonding_curve;
        Some(&self.bonding_curve)
    }

    //make result
    async fn fetch_bounding_curve_data(&self, client: &RpcClient) -> Option<BondingCurve> {
        let data = client
            .get_account_with_commitment(
                &self.accounts.bonding_curve,
                solana_sdk::commitment_config::CommitmentConfig {
                    commitment: solana_sdk::commitment_config::CommitmentLevel::Processed,
                },
            )
            .await
            .ok()?
            .value?
            .data;

        let mut buffer = &data.clone()[..];
        let bonding_curve = BondingCurve::deserialize(&mut buffer).ok();
        bonding_curve
    }

    pub fn bonding_curve(&self) -> &BondingCurve {
        &self.bonding_curve
    }

    pub fn bonding_curve_mut(&mut self) -> &mut BondingCurve {
        &mut self.bonding_curve
    }

    pub fn accounts(&self) -> &TokenAccounts {
        &self.accounts
    }
}
