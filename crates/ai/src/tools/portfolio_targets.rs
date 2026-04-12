//! Portfolio targets tool - get allocation targets and deviation from current portfolio.

use rig::{completion::ToolDefinition, tool::Tool};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::env::AiEnvironment;
use crate::error::AiError;

// ============================================================================
// Tool Arguments and Output
// ============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPortfolioTargetsArgs {
    /// Account ID, or "TOTAL" for the entire portfolio.
    #[serde(default = "default_account_id")]
    pub account_id: String,
}

fn default_account_id() -> String {
    "TOTAL".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingTargetDto {
    pub asset_id: String,
    /// Target percentage within the category (e.g. 30 = 30% of Equity).
    pub target_percent: i32,
    pub is_locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviationDto {
    pub category_id: String,
    pub category_name: String,
    pub target_percent: f64,
    pub current_percent: f64,
    /// Positive = overweight, negative = underweight.
    pub deviation_percent: f64,
    pub current_value: f64,
    pub target_value: f64,
    /// Positive = need to buy more, negative = overweight.
    pub value_delta: f64,
    pub is_locked: bool,
    /// Per-holding targets within this category (empty if none defined).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub holding_targets: Vec<HoldingTargetDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSummaryDto {
    pub target_id: String,
    pub target_name: String,
    pub account_id: String,
    pub taxonomy_id: String,
    pub total_value: f64,
    pub currency: String,
    pub deviations: Vec<DeviationDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPortfolioTargetsOutput {
    pub targets: Vec<TargetSummaryDto>,
}

// ============================================================================
// Tool Implementation
// ============================================================================

pub struct GetPortfolioTargetsTool<E: AiEnvironment> {
    env: Arc<E>,
}

impl<E: AiEnvironment> GetPortfolioTargetsTool<E> {
    pub fn new(env: Arc<E>) -> Self {
        Self { env }
    }
}

impl<E: AiEnvironment> Clone for GetPortfolioTargetsTool<E> {
    fn clone(&self) -> Self {
        Self {
            env: self.env.clone(),
        }
    }
}

impl<E: AiEnvironment + 'static> Tool for GetPortfolioTargetsTool<E> {
    const NAME: &'static str = "get_portfolio_targets";

    type Error = AiError;
    type Args = GetPortfolioTargetsArgs;
    type Output = GetPortfolioTargetsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get portfolio allocation targets and how the current portfolio deviates \
                from them. Returns each target with per-category target%, current%, deviation%, \
                value delta (positive = need to buy more, negative = overweight), and per-holding \
                targets within each category when defined. Use this to answer questions about \
                rebalancing needs or how aligned the portfolio is with its targets."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "accountId": {
                        "type": "string",
                        "description": "Account ID to check targets for, or 'TOTAL' for all accounts combined",
                        "default": "TOTAL"
                    }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target_service = self.env.portfolio_target_service();
        let base_currency = self.env.base_currency();

        let targets = target_service
            .get_targets_by_account(&args.account_id)
            .map_err(|e| AiError::ToolExecutionFailed(e.to_string()))?;

        let mut summaries = Vec::new();

        for target in targets {
            if !target.is_active {
                continue;
            }

            let report = target_service
                .get_deviation_report(&target.id, &base_currency)
                .await
                .map_err(|e| AiError::ToolExecutionFailed(e.to_string()))?;

            // Fetch allocations to get allocation IDs per category (needed for holding targets).
            let allocations = target_service
                .get_allocations_by_target(&target.id)
                .map_err(|e| AiError::ToolExecutionFailed(e.to_string()))?;

            let mut deviations = Vec::new();
            for d in report.deviations {
                // Find the allocation record matching this category to get holding targets.
                let holding_targets = allocations
                    .iter()
                    .find(|a| a.category_id == d.category_id)
                    .map(|a| {
                        target_service
                            .get_holding_targets_by_allocation(&a.id)
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(|h| HoldingTargetDto {
                        asset_id: h.asset_id,
                        target_percent: h.target_percent,
                        is_locked: h.is_locked,
                    })
                    .collect();

                deviations.push(DeviationDto {
                    category_id: d.category_id,
                    category_name: d.category_name,
                    target_percent: d.target_percent.to_f64().unwrap_or(0.0),
                    current_percent: d.current_percent.to_f64().unwrap_or(0.0),
                    deviation_percent: d.deviation_percent.to_f64().unwrap_or(0.0),
                    current_value: d.current_value.to_f64().unwrap_or(0.0),
                    target_value: d.target_value.to_f64().unwrap_or(0.0),
                    value_delta: d.value_delta.to_f64().unwrap_or(0.0),
                    is_locked: d.is_locked,
                    holding_targets,
                });
            }

            summaries.push(TargetSummaryDto {
                target_id: report.target_id,
                target_name: report.target_name,
                account_id: report.account_id,
                taxonomy_id: report.taxonomy_id,
                total_value: report.total_value.to_f64().unwrap_or(0.0),
                currency: base_currency.clone(),
                deviations,
            });
        }

        Ok(GetPortfolioTargetsOutput { targets: summaries })
    }
}
