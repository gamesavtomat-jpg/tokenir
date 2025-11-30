use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};

use crate::pump_interaction::constans;

#[derive(BorshSerialize, BorshDeserialize, Clone)]
pub struct Transfer {
    lamports: u64,
}

impl Transfer {
    const DISCRIMINATOR: [u8; 4] = [2, 0, 0, 0];

    pub fn new(lamports: u64) -> Self {
        Self { lamports }
    }

    pub fn data(&self) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(256);
        buffer.extend_from_slice(&Self::DISCRIMINATOR);
        self.serialize(&mut buffer).unwrap();
        buffer
    }
}

pub fn transfer(from: &Keypair, to: &Pubkey, instruction: &Transfer) -> Instruction {
    Instruction::new_with_bytes(
        constans::programs::SYSTEM_PROGRAM,
        &instruction.data(),
        vec![
            AccountMeta::new(from.pubkey(), true),
            AccountMeta::new(*to, false),
        ],
    )
}
