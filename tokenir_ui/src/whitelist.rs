use solana_sdk::pubkey::Pubkey;
use std::{collections::HashSet, fs, str::FromStr};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum Allowable {
    Twitter(String),
    Wallet(Pubkey),
}

#[derive(Debug, Clone)]
pub struct Whitelist {
    list: HashSet<Allowable>,
}

impl Whitelist {
    pub fn new() -> Self {
        Self {
            list: HashSet::new(),
        }
    }

    pub fn load() -> Self {
        let mut wl = Whitelist::new();

        if let Ok(data) = fs::read_to_string("./whitelist.txt") {
            for line in data.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if let Ok(pk) = Pubkey::from_str(line) {
                    wl.list.insert(Allowable::Wallet(pk));
                } else {
                    wl.list.insert(Allowable::Twitter(line.to_string()));
                }
            }
        } else {
            let _ = wl.to_file();
        }

        wl
    }

    pub fn add(&mut self, target: Allowable) {
        self.list.insert(target);
        let _ = self.to_file();
    }

    pub fn present(&self, target: &Allowable) -> bool {
        self.list.contains(target)
    }

    fn to_file(&self) -> std::io::Result<()> {
        let content = self
            .list
            .iter()
            .map(|e| match e {
                Allowable::Wallet(pk) => pk.to_string(),
                Allowable::Twitter(tw) => tw.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n");

        fs::write("./whitelist.txt", content)
    }
}
