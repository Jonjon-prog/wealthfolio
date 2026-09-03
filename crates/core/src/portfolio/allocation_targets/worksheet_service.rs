//! Orchestration behind the calculated rebalancing worksheet.
//!
//! Implements §4 to §6 of
//! `docs/features/allocations/self-directed-rebalancing-design.md`. The
//! arithmetic lives in [`super::worksheet_calculator`] and stays pure: this
//! file resolves the drift report, the taxonomy contributions, prices, FX and
//! constraints, and hands them over as plain structs.

use async_trait::async_trait;
use chrono::Utc;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet, VecDeque};
use std::str::FromStr;
use std::sync::Arc;

use crate::assets::{Asset, AssetServiceTrait};
use crate::errors::{DatabaseError, Error as CoreError, Result as CoreResult, ValidationError};
use crate::fx::currency::currency_minor_unit;
use crate::fx::{
    denormalization_multiplier, normalize_currency_code, ExchangeRate, FxServiceTrait,
};
use crate::portfolio::allocation::{AllocationServiceTrait, HoldingAllocationContribution};
use crate::portfolio::holdings::{Holding, HoldingType, HoldingsServiceTrait};
use crate::quotes::{LatestQuoteSnapshot, QuoteServiceTrait};

use super::cash::{has_deployable_cash_categories, tracked_cash};
use super::drift_service::DriftServiceTrait;
use super::model::{
    AllocationTargetConstraint, CalculatedAdjustment, CalculatedAdjustments, ConstraintAction,
    ConstraintEffect, ConstraintSubjectType, GenerateCalculatedAdjustmentsInput,
    WorksheetDirection, WorksheetMode, WorksheetPricingSource,
};
use super::target_service::AllocationTargetServiceTrait;
use super::worksheet_calculator::{
    account_funding_shortfalls, apply_limits, assign_accounts, remaining_cash, run_sequence,
    turnover_cap_value, CategoryTarget, LimitsInput, PositionInput, SecurityInput, SequenceInput,
};

const UNKNOWN_CATEGORY_ID: &str = "__UNKNOWN__";

#[async_trait]
pub trait AllocationWorksheetServiceTrait: Send + Sync {
    /// Prefills the worksheet from the target, the eligible securities and the
    /// allocation rule (§4).
    async fn generate_adjustments(
        &self,
        input: GenerateCalculatedAdjustmentsInput,
    ) -> CoreResult<CalculatedAdjustments>;
}

pub struct AllocationWorksheetService {
    allocation_target_service: Arc<dyn AllocationTargetServiceTrait>,
    drift_service: Arc<dyn DriftServiceTrait>,
    allocation_service: Arc<dyn AllocationServiceTrait>,
    holdings_service: Arc<dyn HoldingsServiceTrait>,
    asset_service: Arc<dyn AssetServiceTrait>,
    quote_service: Arc<dyn QuoteServiceTrait>,
    fx_service: Arc<dyn FxServiceTrait>,
}

impl AllocationWorksheetService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        allocation_target_service: Arc<dyn AllocationTargetServiceTrait>,
        drift_service: Arc<dyn DriftServiceTrait>,
        allocation_service: Arc<dyn AllocationServiceTrait>,
        holdings_service: Arc<dyn HoldingsServiceTrait>,
        asset_service: Arc<dyn AssetServiceTrait>,
        quote_service: Arc<dyn QuoteServiceTrait>,
        fx_service: Arc<dyn FxServiceTrait>,
    ) -> Self {
        Self {
            allocation_target_service,
            drift_service,
            allocation_service,
            holdings_service,
            asset_service,
            quote_service,
            fx_service,
        }
    }

    fn invalid(message: impl Into<String>) -> CoreError {
        CoreError::Validation(ValidationError::InvalidInput(message.into()))
    }

    fn asset_key(holding: &Holding) -> String {
        holding
            .instrument
            .as_ref()
            .map(|instrument| instrument.id.clone())
            .unwrap_or_else(|| holding.id.clone())
    }

    fn symbol_of(asset: &Asset) -> String {
        asset
            .display_code
            .clone()
            .or_else(|| asset.instrument_symbol.clone())
            .unwrap_or_else(|| asset.id.clone())
    }

    /// The basis every target weight is sized against.
    ///
    /// §4.2 uses `planning_total` without defining it. Cash the worksheet
    /// deploys joins the classified universe once it is spent, so sizing the
    /// sleeves against a total the worksheet itself grows under-sizes every
    /// target and pushes the surplus onto whatever is already overweight.
    ///
    /// With a cash sleeve the tracked cash is already inside `total_value`, so
    /// only the hypothetical contribution widens the basis.
    ///
    /// The prefill and the preview share this denominator on purpose: a
    /// projection computed against a different total would contradict the
    /// prefill that produced it.
    fn planning_total(
        total_value: Decimal,
        tracked_cash_to_use: Decimal,
        external_cash: Decimal,
        has_cash_sleeve: bool,
    ) -> Decimal {
        if has_cash_sleeve {
            total_value + external_cash
        } else {
            total_value + tracked_cash_to_use + external_cash
        }
    }

    // ── Constraints (#1177) ──────────────────────────────────────────────────

    fn action_matches(direction: &WorksheetDirection, action: &ConstraintAction) -> bool {
        matches!(action, ConstraintAction::Trade)
            || matches!(
                (direction, action),
                (WorksheetDirection::Increase, ConstraintAction::Buy)
                    | (WorksheetDirection::Reduce, ConstraintAction::Sell)
            )
    }

    /// Whether a constraint covers the change being considered.
    ///
    /// `account_id` is `None` where the change has no account yet — the prefill
    /// decides a security's eligibility before §6 places it.
    fn constraint_matches(
        constraint: &AllocationTargetConstraint,
        direction: &WorksheetDirection,
        asset_id: &str,
        account_id: Option<&str>,
        category_ids: &[String],
    ) -> bool {
        if !Self::action_matches(direction, &constraint.action) {
            return false;
        }
        match constraint.subject_type {
            ConstraintSubjectType::Asset => constraint.subject_id == asset_id,
            ConstraintSubjectType::Account => {
                account_id.is_some_and(|id| constraint.subject_id == id)
            }
            ConstraintSubjectType::Category => category_ids.contains(&constraint.subject_id),
        }
    }

    /// A blocking constraint is honoured by the prefill so it never produces a
    /// line the preview would reject. `Avoid` only warns, so it is left to the
    /// preview.
    fn is_blocked(
        constraints: &[AllocationTargetConstraint],
        direction: &WorksheetDirection,
        asset_id: &str,
        account_id: Option<&str>,
        category_ids: &[String],
    ) -> bool {
        constraints.iter().any(|constraint| {
            matches!(constraint.effect, ConstraintEffect::Block)
                && Self::constraint_matches(
                    constraint,
                    direction,
                    asset_id,
                    account_id,
                    category_ids,
                )
        })
    }

    // ── Pricing ──────────────────────────────────────────────────────────────

    fn resolve_fx_source(
        from_currency: &str,
        to_currency: &str,
        rates: &[ExchangeRate],
    ) -> CoreResult<(Decimal, Option<WorksheetPricingSource>, Vec<ExchangeRate>)> {
        let normalized_from = normalize_currency_code(from_currency).to_ascii_uppercase();
        let normalized_to = normalize_currency_code(to_currency).to_ascii_uppercase();
        let source_multiplier = if normalized_from.eq_ignore_ascii_case(from_currency) {
            Decimal::ONE
        } else {
            Decimal::ONE / denormalization_multiplier(from_currency)
        };
        let target_multiplier = denormalization_multiplier(to_currency);
        if normalized_from == normalized_to {
            return Ok((source_multiplier * target_multiplier, None, Vec::new()));
        }

        let mut adjacency = HashMap::<String, Vec<(String, Decimal, usize)>>::new();
        for (index, rate) in rates.iter().enumerate() {
            if rate.rate <= Decimal::ZERO {
                continue;
            }
            let from = normalize_currency_code(&rate.from_currency).to_ascii_uppercase();
            let to = normalize_currency_code(&rate.to_currency).to_ascii_uppercase();
            if from == to {
                continue;
            }
            adjacency
                .entry(from.clone())
                .or_default()
                .push((to.clone(), rate.rate, index));
            adjacency
                .entry(to)
                .or_default()
                .push((from, Decimal::ONE / rate.rate, index));
        }
        for edges in adjacency.values_mut() {
            edges.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| rates[left.2].id.cmp(&rates[right.2].id))
            });
        }

        let mut queue = VecDeque::from([(normalized_from.clone(), Decimal::ONE, Vec::new())]);
        let mut visited = HashSet::from([normalized_from.clone()]);
        let mut resolved = None;
        while let Some((currency, accumulated_rate, path)) = queue.pop_front() {
            if currency == normalized_to {
                resolved = Some((accumulated_rate, path));
                break;
            }
            for (next_currency, edge_rate, rate_index) in
                adjacency.get(&currency).into_iter().flatten()
            {
                if visited.insert(next_currency.clone()) {
                    let mut next_path = path.clone();
                    next_path.push(*rate_index);
                    queue.push_back((
                        next_currency.clone(),
                        accumulated_rate * *edge_rate,
                        next_path,
                    ));
                }
            }
        }

        let (path_rate, path) = resolved.ok_or_else(|| {
            Self::invalid(format!(
                "No attributable FX rate is available for {from_currency}/{to_currency}"
            ))
        })?;
        let used_rates = path
            .into_iter()
            .map(|index| rates[index].clone())
            .collect::<Vec<_>>();
        let applied_rate = source_multiplier * path_rate * target_multiplier;
        let oldest_timestamp = used_rates
            .iter()
            .map(|rate| rate.timestamp)
            .min()
            .ok_or_else(|| Self::invalid("Resolved FX conversion has no source records"))?;
        let is_stale = used_rates
            .iter()
            .any(|rate| rate.timestamp.date_naive() < Utc::now().date_naive());
        let source_id = used_rates
            .iter()
            .map(|rate| rate.id.as_str())
            .collect::<Vec<_>>()
            .join(">");
        Ok((
            applied_rate,
            Some(WorksheetPricingSource {
                id: source_id,
                source_type: if used_rates.len() == 1 {
                    "fx_rate".to_string()
                } else {
                    "fx_path".to_string()
                },
                value: applied_rate,
                from_currency: from_currency.to_string(),
                to_currency: to_currency.to_string(),
                timestamp: oldest_timestamp.to_rfc3339(),
                is_stale,
            }),
            used_rates,
        ))
    }

    /// A unit price in base currency, contract multiplier applied.
    ///
    /// `None` where the preview would refuse to price the line — no snapshot,
    /// no quote, a non-positive close or no attributable FX path — so the
    /// category amount becomes unresolved (§4.4) rather than a line that cannot
    /// be reviewed.
    fn resolved_unit_price(
        asset: &Asset,
        snapshots: &HashMap<String, LatestQuoteSnapshot>,
        base_currency: &str,
        fx_rates: &[ExchangeRate],
    ) -> Option<Decimal> {
        let quote = snapshots.get(&asset.id)?.quote.as_ref()?;
        if quote.close <= Decimal::ZERO {
            return None;
        }
        let (fx_rate, _, _) =
            Self::resolve_fx_source(&quote.currency, base_currency, fx_rates).ok()?;
        let price = quote.close * fx_rate * asset.contract_multiplier();
        (price > Decimal::ZERO).then_some(price)
    }

    // ── Calculator inputs ────────────────────────────────────────────────────

    /// Every recorded security the calculation can act on, with what it is
    /// worth in each category, what a unit costs and where its units sit.
    async fn build_securities(
        &self,
        contributions: &[HoldingAllocationContribution],
        holdings_by_account: &HashMap<String, Vec<Holding>>,
        constraints: &[AllocationTargetConstraint],
        eligible_asset_ids: Option<&HashSet<String>>,
        base_currency: &str,
    ) -> CoreResult<Vec<SecurityInput>> {
        let mut asset_ids: Vec<String> = contributions
            .iter()
            .filter(|contribution| contribution.holding_type != HoldingType::Cash)
            .map(|contribution| contribution.asset_id.clone())
            .collect();
        asset_ids.sort();
        asset_ids.dedup();

        let assets_by_id = self
            .asset_service
            .get_assets_by_asset_ids(&asset_ids)
            .await?
            .into_iter()
            .map(|asset| (asset.id.clone(), asset))
            .collect::<HashMap<_, _>>();

        Ok(Self::securities_from(
            contributions,
            &assets_by_id,
            &self.quote_service.get_latest_quotes_snapshot(&asset_ids)?,
            &self.fx_service.get_latest_exchange_rates()?,
            holdings_by_account,
            constraints,
            eligible_asset_ids,
            base_currency,
        ))
    }

    /// [`build_securities`](Self::build_securities) once every repository has
    /// answered.
    ///
    /// The classification keeps the unclassified residual, so the projection
    /// spreads an amount exactly as the preview's exposures do. Constraints
    /// only ever look at the classified part of it.
    #[allow(clippy::too_many_arguments)]
    fn securities_from(
        contributions: &[HoldingAllocationContribution],
        assets_by_id: &HashMap<String, Asset>,
        snapshots: &HashMap<String, LatestQuoteSnapshot>,
        fx_rates: &[ExchangeRate],
        holdings_by_account: &HashMap<String, Vec<Holding>>,
        constraints: &[AllocationTargetConstraint],
        eligible_asset_ids: Option<&HashSet<String>>,
        base_currency: &str,
    ) -> Vec<SecurityInput> {
        let mut values_by_asset: HashMap<&str, HashMap<&str, Decimal>> = HashMap::new();
        for contribution in contributions
            .iter()
            .filter(|contribution| contribution.holding_type != HoldingType::Cash)
        {
            *values_by_asset
                .entry(contribution.asset_id.as_str())
                .or_default()
                .entry(contribution.category_id.as_str())
                .or_default() += contribution.value;
        }

        let mut asset_ids: Vec<&str> = values_by_asset.keys().copied().collect();
        asset_ids.sort_unstable();

        let mut quantities_by_asset: HashMap<String, HashMap<String, Decimal>> = HashMap::new();
        for (account_id, holdings) in holdings_by_account {
            for holding in holdings.iter().filter(|holding| {
                holding.holding_type != HoldingType::Cash && holding.quantity > Decimal::ZERO
            }) {
                *quantities_by_asset
                    .entry(Self::asset_key(holding))
                    .or_default()
                    .entry(account_id.clone())
                    .or_default() += holding.quantity;
            }
        }

        let mut securities = Vec::new();
        for asset_id in asset_ids {
            let Some(asset) = assets_by_id.get(asset_id) else {
                continue;
            };
            // The preview refuses a line on anything that is not an active
            // tracked investment, so the prefill must not produce one.
            if !asset.is_active || !asset.kind.is_investment() {
                continue;
            }

            let mut category_values: Vec<(String, Decimal)> = values_by_asset
                .get(asset_id)
                .into_iter()
                .flatten()
                .map(|(category_id, value)| (category_id.to_string(), *value))
                .collect();
            category_values.sort_by(|left, right| left.0.cmp(&right.0));
            let classified: Vec<String> = category_values
                .iter()
                .filter(|(category_id, _)| category_id != UNKNOWN_CATEGORY_ID)
                .map(|(category_id, _)| category_id.clone())
                .collect();

            let mut positions: Vec<PositionInput> = quantities_by_asset
                .get(asset_id)
                .into_iter()
                .flatten()
                .map(|(account_id, quantity)| PositionInput {
                    account_id: account_id.clone(),
                    quantity: *quantity,
                    // Eligibility never restricts a reduction (§4.1); the
                    // do-not-sell and avoid-selling constraints do.
                    can_reduce: !Self::is_blocked(
                        constraints,
                        &WorksheetDirection::Reduce,
                        asset_id,
                        Some(account_id),
                        &classified,
                    ),
                })
                .collect();
            positions.sort_by(|left, right| left.account_id.cmp(&right.account_id));

            securities.push(SecurityInput {
                asset_id: asset_id.to_string(),
                symbol: Self::symbol_of(asset),
                unit_price: Self::resolved_unit_price(asset, snapshots, base_currency, fx_rates),
                is_eligible_for_increase: eligible_asset_ids
                    .is_none_or(|eligible| eligible.contains(asset_id))
                    && !Self::is_blocked(
                        constraints,
                        &WorksheetDirection::Increase,
                        asset_id,
                        None,
                        &classified,
                    ),
                category_values,
                positions,
            });
        }

        securities
    }

    /// The accounts each security may be increased in (§6): in scope, and not
    /// blocked from receiving it. Account type, tax wrapper and contribution
    /// room are never inputs.
    fn eligible_accounts(
        securities: &[SecurityInput],
        account_ids: &[String],
        constraints: &[AllocationTargetConstraint],
    ) -> HashMap<String, Vec<String>> {
        securities
            .iter()
            .map(|security| {
                let classified: Vec<String> = security
                    .category_values
                    .iter()
                    .filter(|(category_id, _)| category_id != UNKNOWN_CATEGORY_ID)
                    .map(|(category_id, _)| category_id.clone())
                    .collect();
                let accounts = account_ids
                    .iter()
                    .filter(|account_id| {
                        !Self::is_blocked(
                            constraints,
                            &WorksheetDirection::Increase,
                            &security.asset_id,
                            Some(account_id),
                            &classified,
                        )
                    })
                    .cloned()
                    .collect();
                (security.asset_id.clone(), accounts)
            })
            .collect()
    }
}

#[async_trait]
impl AllocationWorksheetServiceTrait for AllocationWorksheetService {
    async fn generate_adjustments(
        &self,
        input: GenerateCalculatedAdjustmentsInput,
    ) -> CoreResult<CalculatedAdjustments> {
        if input.cash.tracked_cash_to_use < Decimal::ZERO
            || input
                .cash
                .external_contribution
                .values()
                .any(|amount| *amount < Decimal::ZERO)
        {
            return Err(Self::invalid("Worksheet cash values must be non-negative"));
        }
        for account_id in input.cash.external_contribution.keys() {
            if !input.account_ids.contains(account_id) {
                return Err(Self::invalid(format!(
                    "External contribution for account {account_id} is outside the resolved scope"
                )));
            }
        }

        let target = self
            .allocation_target_service
            .get_target(&input.target_id)?
            .ok_or_else(|| {
                CoreError::Database(DatabaseError::NotFound(format!(
                    "AllocationTarget {} not found",
                    input.target_id
                )))
            })?;
        if input.mode == WorksheetMode::Rebalance && !target.allow_sells {
            return Err(Self::invalid(
                "This target disables reductions; enable them before rebalancing",
            ));
        }

        let drift = self
            .drift_service
            .get_drift_report_for_target(
                &input.target_id,
                &input.account_ids,
                &input.base_currency,
                &input.aggregated_account_id,
            )
            .await?;

        let tracked_cash_to_use = if input.cash.tracked_cash_to_use > drift.deployable_cash {
            let overage = input.cash.tracked_cash_to_use - drift.deployable_cash;
            if overage <= currency_minor_unit(&input.base_currency) {
                drift.deployable_cash
            } else {
                return Err(Self::invalid(format!(
                    "Tracked cash selected ({}) exceeds observed deployable cash ({})",
                    input.cash.tracked_cash_to_use, drift.deployable_cash
                )));
            }
        } else {
            input.cash.tracked_cash_to_use
        };

        let contributions = self
            .allocation_service
            .get_holding_contributions_for_taxonomy_for_accounts(
                &input.account_ids,
                &input.base_currency,
                &target.taxonomy_id,
                &input.aggregated_account_id,
            )
            .await?;

        let mut holdings_by_account = HashMap::<String, Vec<Holding>>::new();
        for account_id in &input.account_ids {
            holdings_by_account.insert(
                account_id.clone(),
                self.holdings_service
                    .get_holdings(account_id, &input.base_currency)
                    .await?,
            );
        }

        let constraints = self
            .allocation_target_service
            .list_target_constraints(&input.target_id)?;

        // An empty allowlist is a valid state, not an error (§4.1): every
        // increase it leaves unplaced becomes an unresolved category amount.
        let eligible_asset_ids = input
            .eligible_asset_ids
            .as_ref()
            .map(|ids| ids.iter().cloned().collect::<HashSet<_>>());
        let securities = self
            .build_securities(
                &contributions.contributions,
                &holdings_by_account,
                &constraints,
                eligible_asset_ids.as_ref(),
                &input.base_currency,
            )
            .await?;

        let categories: Vec<CategoryTarget> = drift
            .rows
            .iter()
            .map(|row| CategoryTarget {
                category_id: row.category_id.clone(),
                category_name: row.category_name.clone(),
                target_bps: row.target_bps,
                current_value: row.current_value,
                is_cash: row.is_cash,
            })
            .collect();

        let external_total = input.cash.external_total();
        let planning_total = Self::planning_total(
            drift.total_value,
            tracked_cash_to_use,
            external_total,
            has_deployable_cash_categories(&target.taxonomy_id),
        );

        let sequence = run_sequence(&SequenceInput {
            mode: input.mode.clone(),
            categories: &categories,
            securities: &securities,
            planning_total,
            cash: tracked_cash_to_use,
            external_cash: &input.cash.external_contribution,
            cash_category_id: drift
                .rows
                .iter()
                .find(|row| row.is_cash)
                .map(|row| row.category_id.clone()),
        });

        let min_line_amount = Decimal::from_str(&target.min_trade_amount)
            .unwrap_or(Decimal::ZERO)
            .max(Decimal::ZERO);
        let limited = apply_limits(
            sequence.increases,
            sequence.reductions,
            &securities,
            &LimitsInput {
                tracked_cash: tracked_cash_to_use,
                external_cash: input.cash.external_contribution.clone(),
                turnover_cap: turnover_cap_value(planning_total, target.max_turnover_bps),
                min_line_amount,
                whole_shares_only: target.whole_shares_only,
            },
        );

        let assigned = assign_accounts(
            &limited.lines,
            &securities,
            &Self::eligible_accounts(&securities, &input.account_ids, &constraints),
            target.whole_shares_only,
            min_line_amount,
        );

        // Recorded cash, per account. What the user selected caps the total the
        // limits will spend; this check only answers whether the cash is in the
        // account the increase was placed in (§6).
        let cash_by_account: HashMap<String, Decimal> = holdings_by_account
            .iter()
            .map(|(account_id, holdings)| (account_id.clone(), tracked_cash(holdings)))
            .collect();
        let funding_shortfalls = account_funding_shortfalls(
            &assigned,
            &cash_by_account,
            &input.cash.external_contribution,
        );
        let remaining = remaining_cash(&assigned, tracked_cash_to_use, external_total);

        let symbols: HashMap<&str, &str> = securities
            .iter()
            .map(|security| (security.asset_id.as_str(), security.symbol.as_str()))
            .collect();

        Ok(CalculatedAdjustments {
            mode: input.mode,
            rule: input.rule,
            adjustments: assigned
                .into_iter()
                .map(|line| CalculatedAdjustment {
                    line_id: format!(
                        "calc:{}:{}",
                        line.asset_id,
                        line.account_id.as_deref().unwrap_or("unassigned")
                    ),
                    direction: if line.amount < Decimal::ZERO {
                        WorksheetDirection::Reduce
                    } else {
                        WorksheetDirection::Increase
                    },
                    symbol: symbols
                        .get(line.asset_id.as_str())
                        .map(|symbol| symbol.to_string())
                        .unwrap_or_else(|| line.asset_id.clone()),
                    asset_id: line.asset_id,
                    account_id: line.account_id,
                    amount: line.amount,
                    quantity: line.quantity,
                    unit_price: line.unit_price,
                    is_below_minimum: line.is_below_minimum,
                })
                .collect(),
            unresolved: sequence.unresolved,
            scaling: limited.scaling,
            remaining_cash: remaining,
            funding_shortfalls,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The calculation itself is covered in `worksheet_calculator`, which is pure by
// design. What is left here is the resolution of its inputs: who may be
// increased, who may be reduced, what a unit costs and what the weights are
// sized against.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetKind;
    use crate::portfolio::holdings::{Instrument, MonetaryValue};
    use crate::quotes::Quote;
    use rust_decimal_macros::dec;

    fn asset(id: &str) -> Asset {
        Asset {
            id: id.to_string(),
            kind: AssetKind::Investment,
            display_code: Some(id.to_ascii_uppercase()),
            is_active: true,
            quote_ccy: "USD".to_string(),
            ..Default::default()
        }
    }

    fn assets(ids: &[&str]) -> HashMap<String, Asset> {
        ids.iter().map(|id| (id.to_string(), asset(id))).collect()
    }

    fn contribution(
        asset_id: &str,
        category_id: &str,
        value: Decimal,
    ) -> HoldingAllocationContribution {
        HoldingAllocationContribution {
            id: format!("{asset_id}:{category_id}"),
            holding_id: format!("holding-{asset_id}"),
            asset_id: asset_id.to_string(),
            account_id: "acc-1".to_string(),
            source_account_ids: vec![],
            symbol: asset_id.to_ascii_uppercase(),
            name: asset_id.to_string(),
            holding_type: HoldingType::Security,
            quantity: Decimal::ONE,
            category_id: category_id.to_string(),
            category_name: category_id.to_string(),
            category_color: "#aaa".to_string(),
            value,
        }
    }

    fn holding(asset_id: &str, account_id: &str, quantity: Decimal) -> Holding {
        Holding {
            id: format!("{account_id}-{asset_id}"),
            account_id: account_id.to_string(),
            holding_type: HoldingType::Security,
            is_closed: false,
            instrument: Some(Instrument {
                id: asset_id.to_string(),
                symbol: asset_id.to_ascii_uppercase(),
                name: None,
                currency: "USD".to_string(),
                notes: None,
                pricing_mode: "auto".to_string(),
                preferred_provider: None,
                exchange_mic: None,
                instrument_type: None,
                classifications: None,
            }),
            asset_kind: None,
            quantity,
            open_date: None,
            lots: None,
            contract_multiplier: Decimal::ONE,
            local_currency: "USD".to_string(),
            base_currency: "USD".to_string(),
            fx_rate: None,
            market_value: MonetaryValue {
                local: quantity,
                base: quantity,
            },
            cost_basis: None,
            price: None,
            purchase_price: None,
            unrealized_gain: None,
            unrealized_gain_pct: None,
            realized_gain: None,
            realized_gain_pct: None,
            total_gain: None,
            total_gain_pct: None,
            income: None,
            total_return: None,
            total_return_pct: None,
            return_basis: None,
            day_change: None,
            day_change_pct: None,
            prev_close_value: None,
            weight: Decimal::ZERO,
            as_of_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            metadata: None,
            source_account_ids: vec![],
        }
    }

    fn snapshot(asset_id: &str, close: Decimal, currency: &str) -> LatestQuoteSnapshot {
        LatestQuoteSnapshot {
            quote: Some(Quote {
                id: format!("quote-{asset_id}"),
                asset_id: asset_id.to_string(),
                timestamp: Utc::now(),
                close,
                currency: currency.to_string(),
                data_source: "manual".to_string(),
                ..Default::default()
            }),
            is_stale: false,
            effective_market_date: "2026-01-01".to_string(),
            quote_date: Some("2026-01-01".to_string()),
            no_quote_reason: None,
        }
    }

    fn constraint(
        subject_type: ConstraintSubjectType,
        subject_id: &str,
        action: ConstraintAction,
        effect: ConstraintEffect,
    ) -> AllocationTargetConstraint {
        AllocationTargetConstraint {
            id: format!("constraint-{subject_id}"),
            target_id: "target-1".to_string(),
            subject_type,
            subject_id: subject_id.to_string(),
            action,
            effect,
            reason: None,
            metadata_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// One security worth 1000 of equity, 10 units of it in `acc-1`, priced at
    /// 100 in the base currency.
    fn securities(
        constraints: &[AllocationTargetConstraint],
        eligible: Option<&HashSet<String>>,
    ) -> Vec<SecurityInput> {
        AllocationWorksheetService::securities_from(
            &[contribution("vti", "EQUITY", dec!(1000))],
            &assets(&["vti"]),
            &HashMap::from([("vti".to_string(), snapshot("vti", dec!(100), "USD"))]),
            &[],
            &HashMap::from([("acc-1".to_string(), vec![holding("vti", "acc-1", dec!(10))])]),
            constraints,
            eligible,
            "USD",
        )
    }

    #[test]
    fn the_prefill_and_the_preview_are_sized_against_the_same_total() {
        // With a cash sleeve the tracked cash is already counted; without one,
        // deploying it grows the classified universe.
        assert_eq!(
            AllocationWorksheetService::planning_total(dec!(1000), dec!(200), dec!(100), true),
            dec!(1100)
        );
        assert_eq!(
            AllocationWorksheetService::planning_total(dec!(1000), dec!(200), dec!(100), false),
            dec!(1300)
        );
    }

    #[test]
    fn selecting_no_eligible_security_is_a_valid_state() {
        // §4.1 — the increases that cannot be placed become unresolved category
        // amounts. It is not a validation error, which is what `main` makes it.
        let securities = securities(&[], Some(&HashSet::new()));

        assert_eq!(securities.len(), 1);
        assert!(!securities[0].is_eligible_for_increase);
    }

    #[test]
    fn eligibility_gates_increases_and_never_reductions() {
        let securities = securities(&[], Some(&HashSet::from(["other".to_string()])));

        assert!(!securities[0].is_eligible_for_increase);
        assert!(
            securities[0]
                .positions
                .iter()
                .all(|position| position.can_reduce),
            "an excluded security can still be sold: not adding to it is a different intent"
        );
    }

    #[test]
    fn no_allowlist_means_every_recorded_security() {
        assert!(securities(&[], None)[0].is_eligible_for_increase);
    }

    #[test]
    fn a_do_not_sell_constraint_protects_the_position_it_covers() {
        let blocked = securities(
            &[constraint(
                ConstraintSubjectType::Asset,
                "vti",
                ConstraintAction::Sell,
                ConstraintEffect::Block,
            )],
            None,
        );

        assert!(blocked[0]
            .positions
            .iter()
            .all(|position| !position.can_reduce));
        assert!(
            blocked[0].is_eligible_for_increase,
            "a do-not-sell constraint says nothing about buying"
        );
    }

    #[test]
    fn an_avoid_constraint_leaves_the_prefill_alone() {
        // Avoid warns on the reviewed worksheet; only Block keeps a line from
        // being produced in the first place.
        let securities = securities(
            &[constraint(
                ConstraintSubjectType::Asset,
                "vti",
                ConstraintAction::Trade,
                ConstraintEffect::Avoid,
            )],
            None,
        );

        assert!(securities[0].is_eligible_for_increase);
        assert!(securities[0]
            .positions
            .iter()
            .all(|position| position.can_reduce));
    }

    #[test]
    fn a_category_constraint_covers_the_securities_that_carry_it() {
        let securities = securities(
            &[constraint(
                ConstraintSubjectType::Category,
                "EQUITY",
                ConstraintAction::Buy,
                ConstraintEffect::Block,
            )],
            None,
        );

        assert!(!securities[0].is_eligible_for_increase);
    }

    #[test]
    fn an_account_blocked_from_buying_cannot_receive_an_increase() {
        let constraints = vec![constraint(
            ConstraintSubjectType::Account,
            "acc-2",
            ConstraintAction::Buy,
            ConstraintEffect::Block,
        )];
        let securities = securities(&constraints, None);

        let eligible = AllocationWorksheetService::eligible_accounts(
            &securities,
            &["acc-1".to_string(), "acc-2".to_string()],
            &constraints,
        );

        assert_eq!(eligible["vti"], vec!["acc-1".to_string()]);
    }

    #[test]
    fn a_security_the_preview_would_refuse_is_never_a_candidate() {
        let mut inactive = asset("vti");
        inactive.is_active = false;

        let securities = AllocationWorksheetService::securities_from(
            &[contribution("vti", "EQUITY", dec!(1000))],
            &HashMap::from([("vti".to_string(), inactive)]),
            &HashMap::from([("vti".to_string(), snapshot("vti", dec!(100), "USD"))]),
            &[],
            &HashMap::new(),
            &[],
            None,
            "USD",
        );

        assert!(securities.is_empty());
    }

    #[test]
    fn a_security_without_a_usable_price_carries_none() {
        let securities = AllocationWorksheetService::securities_from(
            &[contribution("vti", "EQUITY", dec!(1000))],
            &assets(&["vti"]),
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &[],
            None,
            "USD",
        );

        assert_eq!(securities[0].unit_price, None);
    }

    #[test]
    fn a_price_is_converted_into_the_base_currency() {
        let rates = vec![ExchangeRate {
            id: "eur-usd".to_string(),
            from_currency: "EUR".to_string(),
            to_currency: "USD".to_string(),
            rate: dec!(1.1),
            source: "manual".to_string(),
            timestamp: Utc::now(),
        }];

        let securities = AllocationWorksheetService::securities_from(
            &[contribution("vti", "EQUITY", dec!(1000))],
            &assets(&["vti"]),
            &HashMap::from([("vti".to_string(), snapshot("vti", dec!(100), "EUR"))]),
            &rates,
            &HashMap::new(),
            &[],
            None,
            "USD",
        );

        assert_eq!(securities[0].unit_price, Some(dec!(110.0)));
    }

    #[test]
    fn the_unclassified_residual_travels_with_the_classification() {
        // The projection has to spread an amount exactly as the preview's
        // exposures do, so the residual is part of the security's value. It
        // carries no target, so no category gap ever reaches it.
        let securities = AllocationWorksheetService::securities_from(
            &[
                contribution("vti", "EQUITY", dec!(650)),
                contribution("vti", UNKNOWN_CATEGORY_ID, dec!(350)),
            ],
            &assets(&["vti"]),
            &HashMap::from([("vti".to_string(), snapshot("vti", dec!(100), "USD"))]),
            &[],
            &HashMap::new(),
            &[],
            None,
            "USD",
        );

        assert_eq!(
            securities[0].category_values,
            vec![
                ("EQUITY".to_string(), dec!(650)),
                (UNKNOWN_CATEGORY_ID.to_string(), dec!(350)),
            ]
        );
    }

    #[test]
    fn a_position_is_where_the_units_actually_sit() {
        let securities = AllocationWorksheetService::securities_from(
            &[contribution("vti", "EQUITY", dec!(1000))],
            &assets(&["vti"]),
            &HashMap::from([("vti".to_string(), snapshot("vti", dec!(100), "USD"))]),
            &[],
            &HashMap::from([
                ("acc-1".to_string(), vec![holding("vti", "acc-1", dec!(6))]),
                ("acc-2".to_string(), vec![holding("vti", "acc-2", dec!(4))]),
            ]),
            &[],
            None,
            "USD",
        );

        let positions = &securities[0].positions;
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].account_id, "acc-1");
        assert_eq!(positions[0].quantity, dec!(6));
        assert_eq!(positions[1].account_id, "acc-2");
        assert_eq!(positions[1].quantity, dec!(4));
    }
}
