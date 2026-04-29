use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};

use crate::{error::ApiResult, main_lib::AppState};
use wealthfolio_core::portfolio::portfolios::{NewPortfolio, Portfolio, PortfolioServiceTrait};

async fn list_portfolios(State(state): State<Arc<AppState>>) -> ApiResult<Json<Vec<Portfolio>>> {
    let portfolios = state.portfolio_service.get_all_portfolios()?;
    Ok(Json(portfolios))
}

async fn get_portfolio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Option<Portfolio>>> {
    let portfolio = state.portfolio_service.get_portfolio(&id)?;
    Ok(Json(portfolio))
}

async fn create_portfolio(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<NewPortfolio>,
) -> ApiResult<(StatusCode, Json<Portfolio>)> {
    let portfolio = state.portfolio_service.create_portfolio(payload).await?;
    Ok((StatusCode::CREATED, Json(portfolio)))
}

async fn update_portfolio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<Portfolio>,
) -> ApiResult<Json<Portfolio>> {
    payload.id = id;
    let portfolio = state.portfolio_service.update_portfolio(payload).await?;
    Ok(Json(portfolio))
}

async fn delete_portfolio(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    state.portfolio_service.delete_portfolio(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn find_portfolio_by_accounts(
    State(state): State<Arc<AppState>>,
    Json(account_ids): Json<Vec<String>>,
) -> ApiResult<Json<Option<Portfolio>>> {
    let portfolio = state.portfolio_service.find_by_accounts(&account_ids)?;
    Ok(Json(portfolio))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/portfolios", get(list_portfolios).post(create_portfolio))
        .route(
            "/portfolios/:id",
            get(get_portfolio)
                .put(update_portfolio)
                .delete(delete_portfolio),
        )
        .route("/portfolios/match", post(find_portfolio_by_accounts))
}
