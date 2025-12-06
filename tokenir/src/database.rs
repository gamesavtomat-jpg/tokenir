use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::{PgPool, Pool, Postgres, Transaction, postgres::PgPoolOptions, prelude::FromRow};

use crate::{Token, access::{AddUserPayload, User}};
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

    // --- NEW: Validate Key ---
    pub async fn validate_user_key(&self, key: &str) -> Result<bool, sqlx::Error> {
        let pool = self.connection();
        
        // Check if access_key exists
        let result: Option<(i32,)> = sqlx::query_as(
            "SELECT id FROM users WHERE access_key = $1"
        )
        .bind(key)
        .fetch_optional(pool)
        .await?;

        Ok(result.is_some())
    }
    // -------------------------

    pub async fn add_user(
        &self,
        caller_admin_key: &str,
        payload: AddUserPayload,
    ) -> Result<(), sqlx::Error> {
        let pool = self.connection();

        // check if caller is admin
        let is_admin: (bool,) = sqlx::query_as(
            "SELECT admin FROM users WHERE access_key = $1"
        )
        .bind(caller_admin_key)
        .fetch_one(pool)
        .await?;

        if payload.provided_key.len() != 32 {
            return Err(sqlx::Error::Protocol("Key must be exactly 32 characters".into()));
        }

        if !is_admin.0 {
            return Err(sqlx::Error::RowNotFound); // not admin
        }

        sqlx::query(
            r#"
            INSERT INTO users (access_key, hint, admin)
            VALUES ($1, $2, false)
            "#,
        )
        .bind(payload.provided_key)
        .bind(payload.hint)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn remove_user(
        &self,
        caller_admin_key: &str,
        user_id: i32,
    ) -> Result<(), sqlx::Error> {
        let pool = self.connection();

        // check if caller is admin
        let is_admin: (bool,) = sqlx::query_as(
            "SELECT admin FROM users WHERE access_key = $1"
        )
        .bind(caller_admin_key)
        .fetch_one(pool)
        .await?;

        if !is_admin.0 {
            return Err(sqlx::Error::RowNotFound); // not admin
        }

        // check if target user is admin
        let target_admin: (bool,) = sqlx::query_as(
            "SELECT admin FROM users WHERE id = $1"
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        if target_admin.0 {
            return Err(sqlx::Error::RowNotFound); // can't delete admin
        }

        sqlx::query(
            "DELETE FROM users WHERE id = $1"
        )
        .bind(user_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn fetch_all_users(
        &self,
        caller_admin_key: &str,
    ) -> Result<Vec<User>, sqlx::Error> {
        let pool = self.connection();

        // check if caller is admin
        let is_admin: (bool,) = sqlx::query_as(
            "SELECT admin FROM users WHERE access_key = $1"
        )
        .bind(caller_admin_key)
        .fetch_one(pool)
        .await?;

        if !is_admin.0 {
            return Err(sqlx::Error::RowNotFound); // not admin
        }

        let users = sqlx::query_as::<_, User>(
            "SELECT id, access_key, hint, admin FROM users"
        )
        .fetch_all(pool)
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

        // create tokens
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tokens (
                mint TEXT PRIMARY KEY,
                dev_address TEXT NOT NULL,
                ath BIGINT NOT NULL DEFAULT 0,
                created_at BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
                CONSTRAINT fk_dev FOREIGN KEY (dev_address) REFERENCES devs(dev_address)
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

    pub fn connection(&self) -> &Pool<Postgres> {
        &self.pool
    }

    pub async fn add_dev(&self, dev: String) -> Result<(), sqlx::Error> {
        let pool = self.connection();
        sqlx::query(
            r#"
            INSERT INTO devs (dev_address, total_token_count)
            VALUES ($1, 1)
            ON CONFLICT (dev_address)
            DO UPDATE SET total_token_count = devs.total_token_count + 1
            "#,
        )
        .bind(dev)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn add_token(
        &self,
        mint: &Pubkey,
        token: &DbToken,
        twitter: String,
    ) -> Result<(), sqlx::Error> {
        let pool = self.connection();
        sqlx::query(
            r#"
            INSERT INTO tokens (mint, dev_address, ath)
            VALUES ($1, $2, $3)
            ON CONFLICT (mint) DO UPDATE SET ath = EXCLUDED.ath
            "#,
        )
        .bind(&mint.to_string())
        .bind(twitter)
        .bind(token.ath) 
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn update_token_ath(
        &self,
        mint: &Pubkey,
        token: &DbToken,
    ) -> Result<(), sqlx::Error> {
        let pool = self.connection();
        
        sqlx::query(
            r#"
            UPDATE tokens
            SET ath = GREATEST(ath, $2)
            WHERE mint = $1
            "#,
        )
        .bind(&mint.to_string())
        .bind(token.ath)
        .execute(pool)
        .await?;
        
        Ok(())
    }

    pub async fn get_tokens_by_dev(&self, dev_address: &str) -> Result<Vec<DbToken>, sqlx::Error> {
        let pool = self.connection();

        let tokens = sqlx::query_as::<_, DbToken>(
            r#"
            SELECT mint, dev_address, ath
            FROM tokens
            WHERE dev_address = $1
            "#,
        )
        .bind(dev_address)
        .fetch_all(pool)
        .await?;

        Ok(tokens)
    }

    pub async fn get_total_coin_count(&self) -> Result<i64, sqlx::Error> {
        let pool = self.connection();

        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM tokens
            "#,
        )
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }
}

#[derive(Debug)]
pub struct Developer {
    pub account: Pubkey,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, FromRow, Serialize, Deserialize)]
pub struct DbToken {
    pub mint: String,
    pub dev_address: String,
    pub ath: i64,
}