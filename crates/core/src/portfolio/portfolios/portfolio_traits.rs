use async_trait::async_trait;

use crate::errors::Result;

use super::portfolio_model::{NewPortfolio, Portfolio};

#[async_trait]
pub trait PortfolioServiceTrait: Send + Sync {
    fn get_all_portfolios(&self) -> Result<Vec<Portfolio>>;
    fn get_portfolio(&self, id: &str) -> Result<Option<Portfolio>>;
    async fn create_portfolio(&self, new: NewPortfolio) -> Result<Portfolio>;
    async fn update_portfolio(&self, portfolio: Portfolio) -> Result<Portfolio>;
    async fn delete_portfolio(&self, id: &str) -> Result<usize>;
    /// Find a portfolio whose account_ids match exactly (order-independent).
    fn find_by_accounts(&self, account_ids: &[String]) -> Result<Option<Portfolio>>;
}

#[async_trait]
pub trait PortfolioRepositoryTrait: Send + Sync {
    fn get_all(&self) -> Result<Vec<Portfolio>>;
    fn get_by_id(&self, id: &str) -> Result<Option<Portfolio>>;
    async fn insert(&self, new: NewPortfolio) -> Result<Portfolio>;
    async fn update(&self, portfolio: Portfolio) -> Result<Portfolio>;
    async fn delete(&self, id: &str) -> Result<usize>;
}
