use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::pump_interaction::constans::{
    self, deriving::associated_token_address, programs::{ASSOCIATED_TOKEN_PROGRAM, SYSTEM_PROGRAM, TOKEN_PROGRAM}
};

pub struct CreateAccount;

impl CreateAccount {
    const DISCRIMINATOR: u8 = 0x1;

    pub fn data() -> Vec<u8> {
        let mut data = Vec::with_capacity(256);
        data.extend_from_slice(&[Self::DISCRIMINATOR]);
        data
    }
}

pub fn create_account(wallet: &Pubkey, mint: &Pubkey, token_2022 : bool) -> Instruction {
    let (ata, _) = associated_token_address(wallet, mint, token_2022);

    let token_program = match token_2022 {
        true => constans::programs::TOKEN_PROGRAM_2022,
        false => constans::programs::TOKEN_PROGRAM,
    };

    Instruction::new_with_bytes(
        ASSOCIATED_TOKEN_PROGRAM,
        &CreateAccount::data(),
        vec![
            AccountMeta::new(*wallet, true),
            AccountMeta::new(ata, false),
            AccountMeta::new(*wallet, true),
            AccountMeta::new(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(token_program, false),
        ],
    )
}
