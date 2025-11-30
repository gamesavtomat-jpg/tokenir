use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};

use crate::{pump_interaction::constans, pump_interaction::wrappers::TokenAccounts};

#[derive(BorshSerialize, BorshDeserialize, Clone)]
pub struct Create {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub creator: Pubkey,
}

impl Create {
    const DISCRIMINATOR: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];

    pub fn new(name: String, symbol: String, uri: String, creator: Pubkey) -> Self {
        Self {
            name,
            symbol,
            uri,
            creator,
        }
    }

    pub fn data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(256);
        data.extend_from_slice(&Self::DISCRIMINATOR);
        self.serialize(&mut data).unwrap();
        data
    }
}

pub fn create(signer: &Keypair, token: &TokenAccounts, instruction: &Create) -> Instruction {
    let mint = *token.mint();

    let bonding_curve = *token.bonding_curve();
    let associated_bonding_curve = *token.associated_bonding_curve();
    let metadata = *token.metadata();

    Instruction::new_with_bytes(
        constans::programs::PUMP_FUN,
        &instruction.data(),
        vec![
            AccountMeta::new(mint, true),
            AccountMeta::new_readonly(constans::accounts::MINT_AUTHORITY, false),
            AccountMeta::new(bonding_curve, false),
            AccountMeta::new(associated_bonding_curve, false),
            AccountMeta::new_readonly(constans::accounts::GLOBAL, false),
            AccountMeta::new_readonly(constans::programs::METAPLEX_PROGRAM, false),
            AccountMeta::new(metadata, false),
            AccountMeta::new(signer.pubkey(), true),
            AccountMeta::new_readonly(constans::programs::SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(constans::programs::TOKEN_PROGRAM, false),
            AccountMeta::new_readonly(constans::programs::ASSOCIATED_TOKEN_PROGRAM, false),
            AccountMeta::new_readonly(constans::programs::RENT_PROGRAM, false),
            AccountMeta::new_readonly(constans::accounts::EVENT_AUTHORITY, false),
            AccountMeta::new_readonly(constans::programs::PUMP_FUN, false),
        ],
    )
}
