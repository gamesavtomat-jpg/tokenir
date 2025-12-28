use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signature::Keypair,
    signer::Signer,
};

use crate::{
    pump_interaction::constans::{
        self,
        accounts::{GLOBAL, PUMP_FEE},
    },
    pump_interaction::wrappers::TokenAccounts,
};

#[derive(BorshSerialize, BorshDeserialize, Clone)]
pub struct Sell {
    token_amount: u64,
    slippage: u64,
}

impl Sell {
    const DISCRIMINATOR: [u8; 8] = [0x33, 0xE6, 0x85, 0xA4, 0x01, 0x7F, 0x83, 0xAD];

    pub fn new(token_amount: u64, slippage: u64) -> Self {
        Self {
            token_amount,
            slippage,
        }
    }

    pub fn data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(256);
        data.extend_from_slice(&Self::DISCRIMINATOR);
        self.serialize(&mut data).unwrap();
        data
    }
}

pub fn sell(signer: &Keypair, token: &TokenAccounts, instruction: &Sell, token_2022 : bool) -> Instruction {
    let mint = *token.mint();

    let bonding_curve = *token.bonding_curve();
    let associated_bonding_curve = *token.associated_bonding_curve();

    let (associated_user_account, _) =
        constans::deriving::associated_token_address(&signer.pubkey(), &mint, token_2022);

    let token_program = match token_2022 {
        true => constans::programs::TOKEN_PROGRAM_2022,
        false => constans::programs::TOKEN_PROGRAM,
    };

    Instruction::new_with_bytes(
        constans::programs::PUMP_FUN,
        &instruction.data(),
        vec![
            AccountMeta::new_readonly(GLOBAL, false),
            AccountMeta::new(PUMP_FEE, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new(bonding_curve, false),
            AccountMeta::new(associated_bonding_curve, false),
            AccountMeta::new(associated_user_account, false),
            AccountMeta::new(signer.pubkey(), true),
            AccountMeta::new_readonly(constans::programs::SYSTEM_PROGRAM, false),
            AccountMeta::new(*token.creator_vault(), false),
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new_readonly(constans::accounts::EVENT_AUTHORITY, false),
            AccountMeta::new_readonly(constans::programs::PUMP_FUN, false),
        ],
    )
}
