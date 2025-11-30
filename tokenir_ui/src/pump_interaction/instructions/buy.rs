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
        deriving::{get_global_volume_accumulator_pda, get_user_volume_accumulator_pda},
    },
    pump_interaction::wrappers::TokenAccounts,
};

#[derive(BorshSerialize, BorshDeserialize, Clone)]
pub struct Buy {
    token_amount: u64,
    slippage: u64,
}

impl Buy {
    const DISCRIMINATOR: [u8; 8] = [0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea];

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

pub fn buy(signer: &Keypair, token: &TokenAccounts, instruction: &Buy) -> Instruction {
    let mint = *token.mint();

    let bonding_curve = *token.bonding_curve();
    let associated_bonding_curve = *token.associated_bonding_curve();

    let (associated_user_account, _) =
        constans::deriving::associated_token_address(&signer.pubkey(), &mint);

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
            AccountMeta::new_readonly(constans::programs::TOKEN_PROGRAM, false),
            AccountMeta::new(*token.creator_vault(), false),
            AccountMeta::new_readonly(constans::accounts::EVENT_AUTHORITY, false),
            AccountMeta::new_readonly(constans::programs::PUMP_FUN, false),
            AccountMeta::new(get_global_volume_accumulator_pda(), false),
            AccountMeta::new(get_user_volume_accumulator_pda(&signer.pubkey()), false),
            AccountMeta::new_readonly(constans::programs::FEE_CONFIG, false),
            AccountMeta::new_readonly(constans::programs::FEE_PROGRAM, false),
        ],
    )
}
