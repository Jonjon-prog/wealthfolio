use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::errors::{Error, Result, ValidationError};

use super::portfolio_model::{NewPortfolio, Portfolio};
use super::portfolio_traits::{PortfolioRepositoryTrait, PortfolioServiceTrait};

pub struct PortfolioService {
    repository: Arc<dyn PortfolioRepositoryTrait>,
}

impl PortfolioService {
    pub fn new(repository: Arc<dyn PortfolioRepositoryTrait>) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl PortfolioServiceTrait for PortfolioService {
    fn get_all_portfolios(&self) -> Result<Vec<Portfolio>> {
        self.repository.get_all()
    }

    fn get_portfolio(&self, id: &str) -> Result<Option<Portfolio>> {
        self.repository.get_by_id(id)
    }

    async fn create_portfolio(&self, new: NewPortfolio) -> Result<Portfolio> {
        validate_portfolio_accounts(&new.account_ids)?;
        self.repository.insert(new).await
    }

    async fn update_portfolio(&self, portfolio: Portfolio) -> Result<Portfolio> {
        validate_portfolio_accounts(&portfolio.account_ids)?;
        self.repository.update(portfolio).await
    }

    async fn delete_portfolio(&self, id: &str) -> Result<usize> {
        self.repository.delete(id).await
    }

    fn find_by_accounts(&self, account_ids: &[String]) -> Result<Option<Portfolio>> {
        if account_ids.len() < 2 {
            return Ok(None);
        }
        let needle: HashSet<&str> = account_ids.iter().map(|s| s.as_str()).collect();
        let portfolios = self.repository.get_all()?;
        Ok(portfolios.into_iter().find(|p| {
            let haystack: HashSet<&str> = p.account_ids.iter().map(|s| s.as_str()).collect();
            haystack == needle
        }))
    }
}

fn validate_portfolio_accounts(account_ids: &[String]) -> Result<()> {
    if account_ids.len() < 2 {
        return Err(Error::Validation(ValidationError::InvalidInput(
            "A portfolio must contain at least 2 accounts.".into(),
        )));
    }
    Ok(())
}
