use std::sync::Arc;

use tauri::State;

use crate::context::ServiceContext;
use wealthfolio_core::portfolio::portfolios::{NewPortfolio, Portfolio};

#[tauri::command]
pub async fn get_portfolios(
    state: State<'_, Arc<ServiceContext>>,
) -> Result<Vec<Portfolio>, String> {
    state
        .portfolio_service
        .get_all_portfolios()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_portfolio_group(
    state: State<'_, Arc<ServiceContext>>,
    id: String,
) -> Result<Option<Portfolio>, String> {
    state
        .portfolio_service
        .get_portfolio(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_portfolio_group(
    state: State<'_, Arc<ServiceContext>>,
    portfolio: NewPortfolio,
) -> Result<Portfolio, String> {
    state
        .portfolio_service
        .create_portfolio(portfolio)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_portfolio_group(
    state: State<'_, Arc<ServiceContext>>,
    portfolio: Portfolio,
) -> Result<Portfolio, String> {
    state
        .portfolio_service
        .update_portfolio(portfolio)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_portfolio_group(
    state: State<'_, Arc<ServiceContext>>,
    id: String,
) -> Result<usize, String> {
    state
        .portfolio_service
        .delete_portfolio(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn find_portfolio_by_accounts(
    state: State<'_, Arc<ServiceContext>>,
    account_ids: Vec<String>,
) -> Result<Option<Portfolio>, String> {
    state
        .portfolio_service
        .find_by_accounts(&account_ids)
        .map_err(|e| e.to_string())
}
