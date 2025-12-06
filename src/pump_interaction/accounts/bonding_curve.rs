use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
pub struct BondingCurve {
    pub discriminator: u64,

    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,

    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,

    pub token_total_supply: u64,
    pub complete: bool,
}

impl BondingCurve {
    pub fn buy(&self, amount: u64) -> Result<u64, &'static str> {
        if self.complete {
            return Err("Curve is complete");
        }

        if amount == 0 {
            return Ok(0);
        }

        // Calculate the product of virtual reserves using u128 to avoid overflow
        let n: u128 = (self.virtual_sol_reserves as u128) * (self.virtual_token_reserves as u128);

        // Calculate the new virtual sol reserves after the purchase
        let i: u128 = (self.virtual_sol_reserves as u128) + (amount as u128);

        // Calculate the new virtual token reserves after the purchase
        let r: u128 = n / i + 1;

        // Calculate the amount of tokens to be purchased
        let s: u128 = (self.virtual_token_reserves as u128) - r;

        // Convert back to u64 and return the minimum of calculated tokens and real reserves
        let s_u64 = s as u64;
        Ok(if s_u64 < self.real_token_reserves {
            s_u64
        } else {
            self.real_token_reserves
        })
    }

    pub fn price(&self, amount: u64, fee_basis_points: Option<u64>) -> Result<u64, &'static str> {
        if self.complete {
            return Err("Curve is complete");
        }

        if amount == 0 {
            return Ok(0);
        }

        let fee_basis_points = fee_basis_points.unwrap_or(100);

        let n: u128 = ((amount as u128) * (self.virtual_sol_reserves as u128))
            / ((self.virtual_token_reserves as u128) + (amount as u128));

        let a: u128 = (n * (fee_basis_points as u128)) / 10000;

        Ok((n - a) as u64)
    }

    pub fn set_reserves(&mut self, sol: u64, token: u64) {
        self.virtual_sol_reserves = sol;
        self.virtual_token_reserves = token;
    }
}

impl Default for BondingCurve {
    fn default() -> Self {
        Self {
            discriminator: 6966180631402821399,
            virtual_token_reserves: 1073000000000000,
            virtual_sol_reserves: 30000000000,
            real_token_reserves: 793100000000000,
            real_sol_reserves: 0,
            token_total_supply: 1000000000000000,
            complete: false,
        }
    }
}
