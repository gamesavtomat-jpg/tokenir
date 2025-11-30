use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};

use std::collections::VecDeque;

use crate::constans::helper::pool_pda;
use crate::{
    DevPerformance, Token, Trade,
    constans::{helper::CommunityInfo, requests::get_user_created_coins},
    database::DbToken,
    filters::FilterSet,
    logs::CreateEvent,
    requests::CreatorHistory,
};

pub struct TokenPool {
    pub filtered: Vec<Pubkey>,
    pub filtered_check: HashSet<Pubkey>,

    pub pool: HashMap<Pubkey, Token>,
    pub history: HashMap<Pubkey, CreatorHistory>,

    pub collector: VecDeque<Pubkey>,
    pub max_size: u64,

    pub filters: FilterSet,
}
#[derive(Debug)]
pub struct TokenNotFoundInPool;

impl TokenPool {
    pub fn new() -> Self {
        Self {
            filtered: vec![],
            pool: HashMap::new(),
            history: HashMap::new(),
            filters: FilterSet::new(),
            filtered_check: HashSet::new(),
            collector: VecDeque::new(),
            max_size: 50,
        }
    }

    pub fn add(&mut self, event: CreateEvent, community: Option<CommunityInfo>) {
        let token = Token::fresh(
            event.name,
            event.symbol,
            event.user,
            event.bonding_curve,
            community,
            event.mint,
        );

        let pda = pool_pda(&event.mint.clone()).0;

        self.pool.insert(pda, token);
        self.collector.push_back(pda);

        if self.collector.len() > self.max_size as usize {
            if let Some(front) = self.collector.front() {
                self.pool.remove(front);
            }
        }
    }

    pub fn clear_migrated(&mut self) {
        self.filtered.clear();
    }

    pub async fn add_dev(&mut self, dev: Pubkey, history: CreatorHistory) {
        self.history.insert(dev, history);
    }

    pub fn attach_dev_performance(
        &mut self,
        mint: &Pubkey,
        average_ath: u64,
        last_tokens: Vec<DbToken>,
        count: usize,
    ) {
        let Some(token) = self.pool.get_mut(mint) else {
            return;
        };

        token.dev_performance = Some(DevPerformance {
            last_tokens,
            average_ath,
            count,
        });
    }

    pub fn update(
        &mut self,
        mint: &Pubkey,
        trade: Trade,
        price: u64,
    ) -> Result<(), TokenNotFoundInPool> {
        let Some(token) = self.pool.get_mut(mint) else {
            return Err(TokenNotFoundInPool);
        };

        token.update(trade, price);
        Ok(())
    }

    pub fn pool(&self) -> &HashMap<Pubkey, Token> {
        &self.pool
    }
}

impl<'a> IntoIterator for &'a TokenPool {
    type Item = (&'a Pubkey, &'a Token);
    type IntoIter = std::collections::hash_map::Iter<'a, Pubkey, Token>;

    fn into_iter(self) -> Self::IntoIter {
        self.pool.iter()
    }
}
