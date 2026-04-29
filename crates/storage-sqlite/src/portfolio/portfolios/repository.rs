use async_trait::async_trait;
use chrono::Utc;
use diesel::prelude::*;
use diesel::r2d2::{self, Pool};
use diesel::SqliteConnection;
use std::sync::Arc;

use wealthfolio_core::portfolio::portfolios::{NewPortfolio, Portfolio, PortfolioRepositoryTrait};
use wealthfolio_core::Result;

use super::model::{NewPortfolioDB, PortfolioDB};
use crate::db::{get_connection, WriteHandle};
use crate::errors::StorageError;
use crate::schema::portfolios;
use crate::schema::portfolios::dsl::*;

pub struct PortfolioRepository {
    pool: Arc<Pool<r2d2::ConnectionManager<SqliteConnection>>>,
    writer: WriteHandle,
}

impl PortfolioRepository {
    pub fn new(
        pool: Arc<Pool<r2d2::ConnectionManager<SqliteConnection>>>,
        writer: WriteHandle,
    ) -> Self {
        Self { pool, writer }
    }
}

#[async_trait]
impl PortfolioRepositoryTrait for PortfolioRepository {
    fn get_all(&self) -> Result<Vec<Portfolio>> {
        let mut conn = get_connection(&self.pool)?;
        let rows = portfolios
            .order(portfolios::name.asc())
            .load::<PortfolioDB>(&mut conn)
            .map_err(StorageError::from)?;
        Ok(rows.into_iter().map(Portfolio::from).collect())
    }

    fn get_by_id(&self, portfolio_id: &str) -> Result<Option<Portfolio>> {
        let mut conn = get_connection(&self.pool)?;
        let row = portfolios
            .find(portfolio_id)
            .first::<PortfolioDB>(&mut conn)
            .optional()
            .map_err(StorageError::from)?;
        Ok(row.map(Portfolio::from))
    }

    async fn insert(&self, new: NewPortfolio) -> Result<Portfolio> {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let row = NewPortfolioDB {
            id: new.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: new.name,
            account_ids: serde_json::to_string(&new.account_ids).unwrap_or_default(),
            created_at: now.clone(),
            updated_at: now,
        };
        let row_id = row.id.clone();
        let pool = self.pool.clone();
        self.writer
            .exec(move |conn| {
                diesel::insert_into(portfolios::table)
                    .values(&row)
                    .execute(conn)
                    .map_err(StorageError::from)?;
                Ok(())
            })
            .await?;

        let mut conn = get_connection(&self.pool)?;
        let inserted = portfolios
            .find(&row_id)
            .first::<PortfolioDB>(&mut conn)
            .map_err(StorageError::from)?;
        let _ = pool; // keep pool alive
        Ok(Portfolio::from(inserted))
    }

    async fn update(&self, portfolio: Portfolio) -> Result<Portfolio> {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let portfolio_id = portfolio.id.clone();
        let row = NewPortfolioDB {
            id: portfolio.id,
            name: portfolio.name,
            account_ids: serde_json::to_string(&portfolio.account_ids).unwrap_or_default(),
            created_at: portfolio.created_at,
            updated_at: now,
        };
        self.writer
            .exec(move |conn| {
                diesel::update(portfolios.find(&row.id))
                    .set(&row)
                    .execute(conn)
                    .map_err(StorageError::from)?;
                Ok(())
            })
            .await?;

        let mut conn = get_connection(&self.pool)?;
        let updated = portfolios
            .find(&portfolio_id)
            .first::<PortfolioDB>(&mut conn)
            .map_err(StorageError::from)?;
        Ok(Portfolio::from(updated))
    }

    async fn delete(&self, portfolio_id: &str) -> Result<usize> {
        let portfolio_id = portfolio_id.to_string();
        self.writer
            .exec(move |conn| {
                let count = diesel::delete(portfolios.find(&portfolio_id))
                    .execute(conn)
                    .map_err(StorageError::from)?;
                Ok(count)
            })
            .await
    }
}
