use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Deserialize)]
pub struct AddUserPayload {
    pub provided_key : String,
    pub hint : String
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: i32,
    pub access_key: String,
    pub hint: String,
    pub admin: bool,
}

