use std::collections::HashSet;

use crate::filter::FilterSet;
use solana_sdk::pubkey::Pubkey;
use tokenir_ui::Token;

pub struct Pool {
    pub feed: Vec<Token>,
    pub feed_check: HashSet<Pubkey>,
    pub filters: FilterSet,
}

impl Pool {
    pub fn new() -> Self {
        Self {
            feed: vec![],
            filters: FilterSet::load("view_filters"),
            feed_check: HashSet::new(),
        }
    }

    pub fn add(&mut self, token: Token) {
        self.feed_check.insert(token.mint.clone());
        self.feed.push(token);
    }

    pub fn clear(&mut self) {
        self.feed_check.clear();
        self.feed.clear();
    }
}
