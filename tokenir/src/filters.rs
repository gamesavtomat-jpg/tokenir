use std::ops::Range;

use crate::Token;
use rust_decimal::prelude::ToPrimitive;

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FilterSet {
    pub filters: HashMap<Tag, Filters>,
}

impl FilterSet {
    pub fn new() -> Self {
        Self {
            filters: HashMap::new(),
        }
    }

    pub fn add_filter(&mut self, tag: Tag, filter: Filters) {
        self.filters.insert(tag, filter);
    }

    pub fn remove_filter(&mut self, tag: &Tag) {
        self.filters.remove(tag);
    }

    pub fn matches(&self, token: &Token, price: u64, average_mcap: u64) -> bool {
        self.filters
            .values()
            .all(|filter| filter.filter(token, average_mcap))
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum Tag {
    AverageDevMarketCap,
    TransactionCount,
}

#[derive(Debug, Clone)]
pub enum Filters {
    AverageDevMarketCap(Range<u64>),
}

impl Filters {
    pub fn filter(&self, token: &Token, average_mcap: u64) -> bool {
        match self {
            Self::AverageDevMarketCap(range) => range.contains(&average_mcap),
        }
    }
}
