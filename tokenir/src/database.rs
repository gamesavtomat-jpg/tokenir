use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::{PgPool, Pool, Postgres, Row, Transaction, postgres::PgPoolOptions, prelude::FromRow};

use crate::{
    Token,
    access::{AddUserPayload, User}, constans::helper::pool_pda,
};

pub struct Database {
    connection_url: String,
    pool: Pool<Postgres>,
}

impl Database {
    pub async fn new(url: String) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(1000)
            .connect(&url)
            .await?;

        Ok(Self {
            pool,
            connection_url: url,
        })
    }

    pub fn connection(&self) -> &Pool<Postgres> {
        &self.pool
    }

    pub async fn get_dev_median_ath(
        &self,
        dev_address: &str,
    ) -> Result<Option<(i64, usize)>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT
                PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ath)::BIGINT AS median,
                COUNT(*)::BIGINT AS count
            FROM tokens
            WHERE dev_address = $1
            "#,
        )
        .bind(dev_address)
        .fetch_one(&self.pool)
        .await?;

        // Use Option to safely handle NULL
        let median: Option<i64> = row.get("median");
        let count: i64 = row.get("count");

        // Wrap in Some, or return None if median is NULL
        Ok(median.map(|m| (m, count as usize)))
    }

    pub async fn get_last_tokens_by_dev(
        &self,
        dev_address: &str,
        limit: i64,
    ) -> Result<Vec<DbToken>, sqlx::Error> {
        let tokens = sqlx::query_as::<_, DbToken>(
            r#"
            SELECT *
            FROM tokens
            WHERE dev_address = $1
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(dev_address)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(tokens)
    }

    pub async fn validate_user_key(&self, key: &str) -> Result<bool, sqlx::Error> {
        let result: Option<(i32,)> = sqlx::query_as("SELECT id FROM users WHERE access_key = $1")
            .bind(key)
            .fetch_optional(self.connection())
            .await?;

        Ok(result.is_some())
    }

    pub async fn add_user(
        &self,
        caller_admin_key: &str,
        payload: AddUserPayload,
    ) -> Result<(), sqlx::Error> {
        let is_admin: (bool,) = sqlx::query_as("SELECT admin FROM users WHERE access_key = $1")
            .bind(caller_admin_key)
            .fetch_one(self.connection())
            .await?;

        if !is_admin.0 {
            return Err(sqlx::Error::RowNotFound);
        }

        if payload.provided_key.len() != 32 {
            return Err(sqlx::Error::Protocol(
                "Key must be exactly 32 characters".into(),
            ));
        }

        sqlx::query(
            r#"
            INSERT INTO users (access_key, hint, admin, autobuy)
            VALUES ($1, $2, false, $3)
            "#,
        )
        .bind(payload.provided_key)
        .bind(payload.hint)
        .bind(payload.autobuy)
        .execute(self.connection())
        .await?;

        Ok(())
    }

    pub async fn get_user_autobuy_status(&self, key: &str) -> Result<bool, sqlx::Error> {
        let result: (bool,) = sqlx::query_as("SELECT autobuy FROM users WHERE access_key = $1")
            .bind(key)
            .fetch_one(self.connection())
            .await?;

        Ok(result.0)
    }

    pub async fn remove_user(
        &self,
        caller_admin_key: &str,
        user_id: i32,
    ) -> Result<(), sqlx::Error> {
        let is_admin: (bool,) = sqlx::query_as("SELECT admin FROM users WHERE access_key = $1")
            .bind(caller_admin_key)
            .fetch_one(self.connection())
            .await?;

        if !is_admin.0 {
            return Err(sqlx::Error::RowNotFound);
        }

        let target_admin: (bool,) = sqlx::query_as("SELECT admin FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(self.connection())
            .await?;

        if target_admin.0 {
            return Err(sqlx::Error::RowNotFound);
        }

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(self.connection())
            .await?;

        Ok(())
    }

    pub async fn fetch_all_users(&self, caller_admin_key: &str) -> Result<Vec<User>, sqlx::Error> {
        let is_admin: (bool,) = sqlx::query_as("SELECT admin FROM users WHERE access_key = $1")
            .bind(caller_admin_key)
            .fetch_one(self.connection())
            .await?;

        if !is_admin.0 {
            return Err(sqlx::Error::RowNotFound);
        }

        let users = sqlx::query_as::<_, User>("SELECT id, access_key, hint, admin, autobuy FROM users")
            .fetch_all(self.connection())
            .await?;

        Ok(users)
    }

    pub async fn initialize_tables(&self) -> Result<(), sqlx::Error> {
        let pool = self.connection();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS devs (
                dev_address TEXT PRIMARY KEY,
                total_token_count INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tokens (
                mint TEXT PRIMARY KEY,
                dev_address TEXT NOT NULL,
                ath BIGINT NOT NULL DEFAULT 0,
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                name TEXT,
                ticker TEXT,
                ipfs TEXT,
                image TEXT,
                description TEXT,
                community_id TEXT,
                CONSTRAINT fk_dev FOREIGN KEY (dev_address)
                    REFERENCES devs(dev_address)
            );
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id SERIAL PRIMARY KEY,
                access_key CHAR(32) UNIQUE,
                hint TEXT,
                admin BOOLEAN DEFAULT false
            );
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO users (access_key, hint, admin)
            VALUES ('af3soy8thnhi06tsqc38talrs4a227ma', 'Админ', true)
            ON CONFLICT (access_key) DO NOTHING;
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn get_key_by_id(&self, user_id: i32) -> Result<String, sqlx::Error> {
        let result: (String,) = sqlx::query_as("SELECT access_key FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(self.connection())
            .await?;

        Ok(result.0)
    }

    pub async fn add_dev(&self, dev: String) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO devs (dev_address, total_token_count)
            VALUES ($1, 1)
            ON CONFLICT (dev_address)
            DO UPDATE SET total_token_count = devs.total_token_count + 1
            "#,
        )
        .bind(dev)
        .execute(self.connection())
        .await?;

        Ok(())
    }

    pub async fn add_token(
        &self,
        mint: &Pubkey,
        token: &DbToken,
        dev_address: String,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.connection().begin().await?;
        println!("{:?}", &token.community_id);
        sqlx::query(
            r#"
            INSERT INTO devs (dev_address, total_token_count)
            VALUES ($1, 1)
            ON CONFLICT (dev_address)
            DO UPDATE SET total_token_count = devs.total_token_count + 1
            "#,
        )
        .bind(&dev_address)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO tokens
                (mint, dev_address, ath, name, ticker, ipfs, image, description, community_id, pool_address)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (mint) DO UPDATE SET
                ath = GREATEST(tokens.ath, EXCLUDED.ath),
                name = COALESCE(NULLIF(EXCLUDED.name, ''), tokens.name),
                ticker = COALESCE(NULLIF(EXCLUDED.ticker, ''), tokens.ticker),
                ipfs = COALESCE(EXCLUDED.ipfs, tokens.ipfs),
                image = COALESCE(EXCLUDED.image, tokens.image),
                description = COALESCE(NULLIF(EXCLUDED.description, ''), tokens.description),
                community_id = COALESCE(NULLIF(EXCLUDED.community_id, ''), tokens.community_id),
                pool_address = COALESCE(NULLIF(EXCLUDED.pool_address, ''), tokens.pool_address)
            "#,
        )
        .bind(mint.to_string())
        .bind(&dev_address)
        .bind(token.ath)
        .bind(&token.name)
        .bind(&token.ticker)
        .bind(&token.ipfs)
        .bind(&token.image)
        .bind(&token.description)
        .bind(&token.community_id)
        .bind(&token.pool_address) // bind new field
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }


    // возвращаем token_any_exists к прежнему виду без community_id
    pub async fn token_any_exists(
        &self,
        name: Option<&str>,
        ticker: Option<&str>,
        ipfs: Option<&str>,
        image: Option<&str>,
        description: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(bool,)> = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM tokens
                WHERE ($1 IS NOT NULL AND image = $1)
                   OR ($2 IS NOT NULL AND ipfs = $2)
                   OR ($3 IS NOT NULL AND $6 IS NOT NULL AND description = $3 AND name = $6)
                   OR ($3 IS NOT NULL AND $7 IS NOT NULL AND description = $3 AND ticker = $7)
                   OR ($4 IS NOT NULL AND $5 IS NOT NULL AND name = $4 AND ticker = $5)
                   OR ($4 IS NOT NULL AND EXISTS(SELECT 1 FROM tokens WHERE name = $4))
            )
            "#,
        )
        .bind(image) // $1
        .bind(ipfs) // $2
        .bind(description) // $3
        .bind(name) // $4
        .bind(ticker) // $5
        .bind(name) // $6 for (description + name)
        .bind(ticker) // $7 for (description + ticker)
        .fetch_optional(self.connection())
        .await?;

        Ok(row.map(|r| r.0).unwrap_or(false))
    }

    pub async fn get_last_tokens_by_dev_excluding(
        &self,
        dev_address: &str,
        exclude_mint: &str,
        limit: i64,
    ) -> Result<Vec<DbToken>, sqlx::Error> {
        let tokens = sqlx::query_as::<_, DbToken>(
            r#"
            SELECT *
            FROM tokens
            WHERE dev_address = $1 AND mint != $2
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(dev_address)
        .bind(exclude_mint)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(tokens)
    }

    // Updated median calculation that excludes a specific mint
    pub async fn get_dev_median_ath_excluding(
        &self,
        dev_address: &str,
        exclude_mint: &str,
    ) -> Result<Option<(i64, usize)>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT
                PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ath)::BIGINT AS median,
                COUNT(*)::BIGINT AS count
            FROM tokens
            WHERE dev_address = $1 AND mint != $2
            "#,
        )
        .bind(dev_address)
        .bind(exclude_mint)
        .fetch_one(&self.pool)
        .await?;

        let median: Option<i64> = row.get("median");
        let count: i64 = row.get("count");

        Ok(median.map(|m| (m, count as usize)))
    }        

    pub async fn token_community_exists(&self, community_id: &str) -> Result<bool, sqlx::Error> {
        let row: Option<(bool,)> = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM tokens
                WHERE community_id = $1
            )
            "#,
        )
        .bind(community_id)
        .fetch_optional(self.connection())
        .await?;

        Ok(row.map(|r| r.0).unwrap_or(false))
    }

    pub async fn update_token_ath(
        &self,
        pool_address: &Pubkey,
        ath : i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE tokens
            SET ath = GREATEST(ath, $2)
            WHERE pool_address = $1
            "#,
        )
        .bind(pool_address.to_string())
        .bind(ath)
        .execute(self.connection())
        .await?;

        Ok(())
    }

    pub async fn get_tokens_by_dev(&self, dev_address: &str) -> Result<Vec<DbToken>, sqlx::Error> {
        let tokens = sqlx::query_as::<_, DbToken>(
            r#"
            SELECT
                mint,
                dev_address,
                ath,
                COALESCE(name, 'No name found') AS name,
                COALESCE(ticker, 'No ticker found') AS ticker,
                ipfs,
                image,
                description,
                community_id,
                pool_address
            FROM tokens
            WHERE dev_address = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(dev_address)
        .fetch_all(self.connection())
        .await?;

        Ok(tokens)
    }


    pub async fn get_total_coin_count(&self) -> Result<i64, sqlx::Error> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tokens")
            .fetch_one(self.connection())
            .await?;

        Ok(count.0)
    }
}

#[derive(Clone, Debug, FromRow, Serialize, Deserialize)]
pub struct DbToken {
    pub mint: String,
    pub dev_address: String,
    pub ath: i64,
    pub name: String,
    pub ticker: String,
    pub ipfs: Option<String>,
    pub image: Option<String>,
    pub description: Option<String>,
    pub community_id: Option<String>,
    pub pool_address : String
}
