use crate::{
    Token,
    database::{Database, DbToken},
};
use solana_sdk::pubkey::Pubkey;
use sqlx::{Pool, Postgres};
use std::collections::HashMap; // Ensure this import is present

pub struct Bundler {
    limit: u64,
    current: u64,
    data: HashMap<Pubkey, DbToken>,
}

impl Bundler {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            current: 0,
            data: HashMap::new(),
        }
    }

    pub fn add(&mut self, data: (Pubkey, DbToken)) {
        self.current += 1;
        self.data.insert(data.0, data.1);
    }

    pub fn full(&self) -> bool {
        self.current >= self.limit
    }

    pub async fn send(&mut self, database: &Database) -> Result<(), sqlx::Error> {
        // if self.data.is_empty() {
        //     return Ok(());
        // }

        // self.data.clear();
        // self.current = 0;

        Ok(())
    }
}
