pub mod programs {
    use solana_sdk::pubkey;
    use solana_sdk::pubkey::Pubkey;

    pub const SYSTEM_PROGRAM: Pubkey = pubkey!("11111111111111111111111111111111");
    pub const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
    pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
    pub const RENT_PROGRAM: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");

    pub const PUMP_FUN: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
    pub const METAPLEX_PROGRAM: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

    pub const FEE_CONFIG: Pubkey = pubkey!("8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt");
    pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
}

pub mod accounts {
    use solana_sdk::pubkey;
    use solana_sdk::pubkey::Pubkey;

    pub const EVENT_AUTHORITY: Pubkey = pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");
    pub const MINT_AUTHORITY: Pubkey = pubkey!("TSLvdd1pWpHVjahSpsvCXUbgwsL3JAcvokwaKt1eokM");
    pub const GLOBAL: Pubkey = pubkey!("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf");

    pub const PUMP_FEE: Pubkey = pubkey!("G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP");
}

pub mod deriving {
    use crate::pump_interaction::constans::programs;
    use solana_sdk::pubkey::Pubkey;

    pub fn bounding_curve(mint: &Pubkey) -> (Pubkey, u8) {
        let seeds = &[b"bonding-curve", mint.as_ref()];
        Pubkey::find_program_address(seeds, &programs::PUMP_FUN)
    }

    pub fn associated_token_address(wallet: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
        let seeds = &[
            wallet.as_ref(),
            &programs::TOKEN_PROGRAM.to_bytes(),
            mint.as_ref(),
        ];
        Pubkey::find_program_address(seeds, &programs::ASSOCIATED_TOKEN_PROGRAM)
    }

    pub fn creator_vault(creator: &Pubkey) -> (Pubkey, u8) {
        let seeds = &[b"creator-vault", creator.as_ref()];
        Pubkey::find_program_address(seeds, &programs::PUMP_FUN)
    }

    pub fn metadata(mint: &Pubkey) -> (Pubkey, u8) {
        let seeds = &[
            b"metadata",
            programs::METAPLEX_PROGRAM.as_ref(),
            mint.as_ref(),
        ];
        Pubkey::find_program_address(seeds, &programs::METAPLEX_PROGRAM)
    }

    /// Returns the PDA of the global volume accumulator account.
    ///
    /// # Returns
    /// Constant PDA of the global volume accumulator.
    pub fn get_global_volume_accumulator_pda() -> Pubkey {
        let (global_volume_accumulator, _bump) = Pubkey::find_program_address(
            &[b"global_volume_accumulator"],
            &Pubkey::from_str_const("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        );
        global_volume_accumulator
    }

    /// Returns the PDA of a user volume accumulator account.
    ///
    /// # Arguments
    /// * `user` - Public key of the user.
    ///
    /// # Returns
    /// PDA of the corresponding user volume accumulator account.
    pub fn get_user_volume_accumulator_pda(user: &Pubkey) -> Pubkey {
        let (user_volume_accumulator, _bump) = Pubkey::find_program_address(
            &[b"user_volume_accumulator", user.as_ref()],
            &Pubkey::from_str_const("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        );
        user_volume_accumulator
    }
}
