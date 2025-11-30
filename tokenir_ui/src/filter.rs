use std::collections::HashMap;
use std::ops::Range;

use serde_json::to_string;
use tokenir_ui::Token;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct FilterSet {
    pub filters: HashMap<Tag, Filters>,
}

impl FilterSet {
    pub fn new() -> Self {
        Self {
            filters: HashMap::new(),
        }
    }

    pub fn to_file(&self, name: &str) -> Result<(), std::io::Error> {
        let _ = std::fs::write(format!("./{}.json", name), to_string(self).unwrap())?;
        Ok(())
    }

    pub fn load(name: &str) -> FilterSet {
        match std::fs::read_to_string(format!("./{}.json", name)) {
            Ok(data) => {
                let blacklist: Self = serde_json::from_str(&data).unwrap_or(FilterSet::new());
                let _ = blacklist.to_file(name);
                blacklist
            }

            Err(_) => {
                let blacklist = FilterSet::new();
                let _ = blacklist.to_file(name);
                blacklist
            }
        }
    }

    pub fn add_filter(&mut self, tag: Tag, filter: Filters) {
        self.filters.insert(tag, filter);
    }

    pub fn remove_filter(&mut self, tag: &Tag) {
        self.filters.remove(tag);
    }

    pub fn matches(&self, token: &Token, average_mcap: Option<u64>) -> bool {
        let mut mcap_pass = None;
        let mut migration_pass = None;
        let mut token_count_pass = None;

        for (tag, filter) in &self.filters {
            match (tag, filter) {
                (Tag::AverageDevMarketCap, Filters::AverageDevMarketCap(_)) => {
                    if average_mcap.is_none() {
                        mcap_pass = Some(false);
                    } else {
                        mcap_pass = Some(filter.filter(token, average_mcap.unwrap_or(0)));
                    }
                }
                (Tag::MigrationPercentage, Filters::MigrationPercentage(_)) => {
                    migration_pass = Some(filter.filter(token, average_mcap.unwrap_or(0)));
                }
                (Tag::TokenCount, Filters::TokenCount(_)) => {
                    token_count_pass = Some(filter.filter(token, average_mcap.unwrap_or(0)));
                }

                _ => (),
            }
        }

        let mcap_ok = mcap_pass.unwrap_or(false);
        let migration_ok = migration_pass.unwrap_or(false);
        let token_count_ok = token_count_pass.unwrap_or(false);

        let result = mcap_ok || (migration_ok && token_count_ok);
        result
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq, Hash, Clone)]
pub enum Tag {
    AverageDevMarketCap,
    MigrationPercentage,
    TokenCount,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum Filters {
    AverageDevMarketCap(Range<u64>),
    TokenCount(Range<u64>),
    MigrationPercentage(Range<u64>),
}

impl Filters {
    pub fn filter(&self, token: &Token, average_mcap: u64) -> bool {
        match self {
            Self::AverageDevMarketCap(range) => range.contains(&average_mcap),

            Self::TokenCount(range) => {
                if let Some(history) = &token.migrated {
                    return range.contains(&history.counts.totalCount);
                }

                false
            }

            Self::MigrationPercentage(range) => {
                if let Some(history) = &token.migrated {
                    let percentage = ((history.counts.migratedCount as f32
                        / history.counts.totalCount as f32)
                        * 100f32)
                        .floor() as u64;

                    return range.contains(&percentage);
                }

                return false;
            }
        }
    }
}
