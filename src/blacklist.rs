use std::{collections::HashSet, fs};

use serde::{Deserialize, Serialize};
use serde_json::to_string;
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Bannable {
    Twitter(String),
    Wallet(Pubkey),
}

#[derive(Debug, Serialize, Deserialize)]

pub struct Blacklist {
    list: HashSet<Bannable>,
}

impl Blacklist {
    pub fn new() -> Blacklist {
        Self {
            list: HashSet::new(),
        }
    }

    pub fn load() -> Blacklist {
        match fs::read_to_string("./blacklist.json") {
            Ok(data) => {
                let blacklist: Self = serde_json::from_str(&data).unwrap_or(Blacklist::new());
                let _ = blacklist.to_file();
                blacklist
            }

            Err(_) => {
                let blacklist = Blacklist::new();
                let _ = blacklist.to_file();
                blacklist
            }
        }
    }

    pub fn add(&mut self, target: Bannable) {
        self.list.insert(target);

        match self.to_file() {
            Err(err) => eprintln!("{err}"),
            _ => (),
        }
    }

    pub fn present(&self, target: &Bannable) -> bool {
        self.list.contains(&target)
    }

    fn to_file(&self) -> Result<(), std::io::Error> {
        let _ = fs::write("./blacklist.json", to_string(self).unwrap())?;
        Ok(())
    }
}
