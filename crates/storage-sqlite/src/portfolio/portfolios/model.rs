use diesel::prelude::*;
use wealthfolio_core::portfolio::portfolios::Portfolio;

use crate::schema::portfolios;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = portfolios)]
pub struct PortfolioDB {
    pub id: String,
    pub name: String,
    pub account_ids: String, // JSON array
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = portfolios)]
pub struct NewPortfolioDB {
    pub id: String,
    pub name: String,
    pub account_ids: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<PortfolioDB> for Portfolio {
    fn from(db: PortfolioDB) -> Self {
        let account_ids: Vec<String> = serde_json::from_str(&db.account_ids).unwrap_or_default();
        Self {
            id: db.id,
            name: db.name,
            account_ids,
            created_at: db.created_at,
            updated_at: db.updated_at,
        }
    }
}
