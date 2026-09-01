//! Arithmetic behind the calculated rebalancing worksheet.
//!
//! Implements §4 of `docs/features/allocations/self-directed-rebalancing-design.md`.
//! Everything here is a pure function over inputs the service has already
//! resolved: no repositories, no clock, no currency conversion. The service
//! fetches the drift report, the taxonomy contributions, prices and
//! constraints, and hands them over as the structs below.
//!
//! The product boundary this file has to hold: it spreads amounts the user's
//! own target implies, over securities the user marked eligible, by a rule the
//! user picked. It never ranks, scores or chooses between securities.

use std::collections::HashMap;

use rust_decimal::Decimal;

use super::model::{
    AdjustmentScaling, UnresolvedCategoryAmount, UnresolvedReason, WorksheetDirection,
};

/// A target category as the calculation sees it, taken from the drift report.
#[derive(Debug, Clone)]
pub struct CategoryTarget {
    pub category_id: String,
    pub category_name: String,
    pub target_bps: i32,
    pub current_value: Decimal,
}

/// Where one security's units sit, and whether they may be reduced.
#[derive(Debug, Clone)]
pub struct PositionInput {
    pub account_id: String,
    pub quantity: Decimal,
    /// False when a do-not-sell or avoid-selling constraint from #1177 covers
    /// this position. Eligibility never sets this — it gates increases only.
    pub can_reduce: bool,
}

/// A recorded security: what it is worth in each category, what a unit costs,
/// and where its units sit.
#[derive(Debug, Clone)]
pub struct SecurityInput {
    pub asset_id: String,
    pub symbol: String,
    /// Base currency, contract multiplier already applied. `None` when no
    /// usable price could be resolved.
    pub unit_price: Option<Decimal>,
    /// Chosen by the user as able to receive increases (§4.1).
    pub is_eligible_for_increase: bool,
    /// Value attributed to each category, in base currency. A security
    /// classified across several categories appears once per category.
    pub category_values: Vec<(String, Decimal)>,
    pub positions: Vec<PositionInput>,
}

impl SecurityInput {
    fn value_in(&self, category_id: &str) -> Decimal {
        self.category_values
            .iter()
            .find(|(id, _)| id == category_id)
            .map(|(_, value)| *value)
            .unwrap_or(Decimal::ZERO)
    }

    fn reducible_quantity(&self) -> Decimal {
        self.positions
            .iter()
            .filter(|position| position.can_reduce)
            .map(|position| position.quantity)
            .sum()
    }
}

/// One category's share of what a security should be adjusted by, before the
/// per-security amounts are combined (§4.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    pub asset_id: String,
    pub category_id: String,
    /// Signed: positive increases the position, negative reduces it.
    pub amount: Decimal,
}

/// The gap between a category's target value and what it currently holds
/// (§4.2). Positive means underweight, so the category wants an increase.
///
/// `planning_total` rather than the drift report's total, because the two
/// differ when the taxonomy has no cash sleeve and the cash being deployed
/// widens the basis.
pub fn category_gaps(
    categories: &[CategoryTarget],
    planning_total: Decimal,
) -> Vec<(String, Decimal)> {
    categories
        .iter()
        .map(|category| {
            let target_value =
                Decimal::from(category.target_bps) / Decimal::from(10_000) * planning_total;
            (
                category.category_id.clone(),
                target_value - category.current_value,
            )
        })
        .collect()
}

/// Spreads one category's gap across the securities that carry it, in
/// proportion to what each already holds of that category (§4.2).
///
/// ```text
/// weight_i    = value_i,c / Σ_j value_j,c
/// intent_i,c  = category_gap(c) × weight_i
/// ```
///
/// Returns the reason instead when the gap cannot be placed at all, which the
/// caller turns into an unresolved category amount (§4.4).
pub fn split_gap(
    category_id: &str,
    gap: Decimal,
    securities: &[SecurityInput],
) -> Result<Vec<Intent>, UnresolvedReason> {
    if gap == Decimal::ZERO {
        return Ok(Vec::new());
    }

    let direction = if gap > Decimal::ZERO {
        WorksheetDirection::Increase
    } else {
        WorksheetDirection::Reduce
    };

    let exposed: Vec<&SecurityInput> = securities
        .iter()
        .filter(|security| security.value_in(category_id) > Decimal::ZERO)
        .collect();
    if exposed.is_empty() {
        return Err(UnresolvedReason::NoRecordedSecurity);
    }

    // Eligibility gates increases and never restricts reductions (§4.1). A
    // reduction needs somewhere to draw from instead, so a position covered by
    // a do-not-sell constraint is not a carrier either.
    let permitted: Vec<&SecurityInput> = exposed
        .into_iter()
        .filter(|security| match direction {
            WorksheetDirection::Increase => security.is_eligible_for_increase,
            WorksheetDirection::Reduce => security.reducible_quantity() > Decimal::ZERO,
        })
        .collect();
    if permitted.is_empty() {
        return Err(UnresolvedReason::NoEligibleSecurity);
    }

    let carriers: Vec<&SecurityInput> = permitted
        .into_iter()
        .filter(|security| {
            security
                .unit_price
                .is_some_and(|price| price > Decimal::ZERO)
        })
        .collect();
    if carriers.is_empty() {
        return Err(UnresolvedReason::NoUsablePrice);
    }

    let total: Decimal = carriers
        .iter()
        .map(|security| security.value_in(category_id))
        .sum();
    // Allocating by current holding proportions needs proportions to work
    // from. Carriers worth nothing give none, and inventing a split between
    // them would be the app choosing between securities.
    if total <= Decimal::ZERO {
        return Err(UnresolvedReason::NoRecordedSecurity);
    }

    Ok(carriers
        .iter()
        .map(|security| Intent {
            asset_id: security.asset_id.clone(),
            category_id: category_id.to_string(),
            amount: gap * security.value_in(category_id) / total,
        })
        .collect())
}

/// Combines every category's intent for a security into the one amount that
/// gets applied to it (§4.5).
///
/// A security classified 60 % equity / 40 % fixed income moves both categories
/// once, by the amount actually applied to it — so the projection is
/// recalculated from these combined amounts, never from the intents that
/// produced them.
pub fn combine_intents(intents: &[Intent]) -> HashMap<String, Decimal> {
    let mut combined: HashMap<String, Decimal> = HashMap::new();
    for intent in intents {
        *combined.entry(intent.asset_id.clone()).or_default() += intent.amount;
    }
    combined
}

/// Runs §4.2 over every category, keeping the intents that placed and the
/// amounts that did not.
pub fn spread_gaps(
    categories: &[CategoryTarget],
    planning_total: Decimal,
    securities: &[SecurityInput],
) -> (Vec<Intent>, Vec<UnresolvedCategoryAmount>) {
    let mut intents = Vec::new();
    let mut unresolved = Vec::new();

    for (category_id, gap) in category_gaps(categories, planning_total) {
        match split_gap(&category_id, gap, securities) {
            Ok(placed) => intents.extend(placed),
            Err(reason) => {
                let category = categories
                    .iter()
                    .find(|candidate| candidate.category_id == category_id);
                unresolved.push(UnresolvedCategoryAmount {
                    category_id: category_id.clone(),
                    category_name: category
                        .map(|category| category.category_name.clone())
                        .unwrap_or_else(|| category_id.clone()),
                    amount: gap,
                    reason,
                });
            }
        }
    }

    (intents, unresolved)
}

// ── Limits (§4.6) ────────────────────────────────────────────────────────────
//
// Applied in the order below before the worksheet is prefilled. Every step
// either scales proportionally or reports; none of them drops a line, since
// dropping would be a choice between securities.

/// Which funding an increase depends on (§4.6 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncreaseFunding {
    /// Sized against the cash deployed in the first pass. A shortfall in
    /// reduction proceeds does not belong to it, so it is never scaled.
    Cash,
    /// Depends on what the reductions raise, so it absorbs any shortfall.
    Proceeds,
}

/// An increase before the limits are applied. Amount is positive.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftIncrease {
    pub asset_id: String,
    pub amount: Decimal,
    pub funding: IncreaseFunding,
}

/// A reduction before the limits are applied. Amount is a positive magnitude.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftReduction {
    pub asset_id: String,
    pub amount: Decimal,
}

/// What the user's target and guardrails allow, in base currency.
#[derive(Debug, Clone)]
pub struct LimitsInput {
    /// Tracked cash the user selected for deployment.
    pub tracked_cash: Decimal,
    /// Hypothetical cash not currently recorded.
    pub external_cash: Decimal,
    /// Absolute cap on the reduction total. Derive it from the target's
    /// `max_turnover_bps` with [`turnover_cap_value`].
    pub turnover_cap: Option<Decimal>,
    pub min_line_amount: Decimal,
    pub whole_shares_only: bool,
}

/// One line after the limits are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitedLine {
    pub asset_id: String,
    /// Signed: negative for a reduction.
    pub amount: Decimal,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    /// Below the target's minimum — reported, never dropped, never re-rounded
    /// to lift it over the threshold (§4.6 step 6).
    pub is_below_minimum: bool,
}

#[derive(Debug, Clone)]
pub struct LimitedAdjustments {
    pub increases: Vec<LimitedLine>,
    pub reductions: Vec<LimitedLine>,
    pub scaling: AdjustmentScaling,
    /// Left after rounding and minimum-line reporting. Never redistributed.
    pub remaining_cash: Decimal,
}

/// The turnover cap as an amount, from the target's `max_turnover_bps`.
pub fn turnover_cap_value(
    planning_total: Decimal,
    max_turnover_bps: Option<i32>,
) -> Option<Decimal> {
    max_turnover_bps.map(|bps| planning_total * Decimal::from(bps) / Decimal::from(10_000))
}

fn unit_price_of(securities: &[SecurityInput], asset_id: &str) -> Option<Decimal> {
    securities
        .iter()
        .find(|security| security.asset_id == asset_id)
        .and_then(|security| security.unit_price)
        .filter(|price| *price > Decimal::ZERO)
}

/// §4.6 step 1 — a reduction cannot exceed the recorded position it is drawn
/// from. Positions a do-not-sell constraint covers are not drawn from at all.
fn cap_to_held_quantity(reductions: &mut [DraftReduction], securities: &[SecurityInput]) {
    for reduction in reductions.iter_mut() {
        let Some(security) = securities
            .iter()
            .find(|security| security.asset_id == reduction.asset_id)
        else {
            reduction.amount = Decimal::ZERO;
            continue;
        };
        let held_value =
            security.unit_price.unwrap_or(Decimal::ZERO) * security.reducible_quantity();
        reduction.amount = reduction.amount.min(held_value);
    }
}

/// §4.6 step 2 — every reduction is scaled by the same factor to fit the cap.
///
/// Returns the factor when the cap bound, so the result can state that the
/// scaling was applied.
fn scale_to_turnover_cap(
    reductions: &mut [DraftReduction],
    cap: Option<Decimal>,
) -> Option<Decimal> {
    let cap = cap?;
    let total: Decimal = reductions.iter().map(|reduction| reduction.amount).sum();
    if total <= cap || total <= Decimal::ZERO {
        return None;
    }

    let factor = cap / total;
    for reduction in reductions.iter_mut() {
        reduction.amount *= factor;
    }
    Some(factor)
}

/// §4.6 step 4 — increases are scaled down to the funding available to them.
///
/// Increases already covered by cash deployed in the first pass are left alone:
/// the shortfall belongs to the increases that depend on reduction proceeds, so
/// scaling the cash-funded ones would leave selected cash undeployed for no
/// reason.
fn scale_to_funding(increases: &mut [DraftIncrease], available: Decimal) -> Option<Decimal> {
    let total: Decimal = increases.iter().map(|increase| increase.amount).sum();
    if total <= available {
        return None;
    }

    let proceeds_funded: Decimal = increases
        .iter()
        .filter(|increase| increase.funding == IncreaseFunding::Proceeds)
        .map(|increase| increase.amount)
        .sum();
    if proceeds_funded <= Decimal::ZERO {
        return None;
    }

    // Clamped: a shortfall deeper than the proceeds-funded total would mean the
    // first pass sized increases beyond the cash it had, which it does not do.
    let factor = ((proceeds_funded - (total - available)) / proceeds_funded).max(Decimal::ZERO);
    for increase in increases.iter_mut() {
        if increase.funding == IncreaseFunding::Proceeds {
            increase.amount *= factor;
        }
    }
    Some(factor)
}

/// §4.6 steps 5 and 6 — quantities are floored under whole-unit policy, and a
/// line below the minimum is flagged rather than dropped or re-rounded.
fn finalize(
    asset_id: &str,
    amount: Decimal,
    unit_price: Decimal,
    direction: WorksheetDirection,
    limits: &LimitsInput,
) -> LimitedLine {
    let mut quantity = amount / unit_price;
    if limits.whole_shares_only {
        quantity = quantity.floor();
    }
    let resolved = quantity * unit_price;
    let is_below_minimum =
        limits.min_line_amount > Decimal::ZERO && resolved < limits.min_line_amount;
    let sign = match direction {
        WorksheetDirection::Increase => Decimal::ONE,
        WorksheetDirection::Reduce => Decimal::NEGATIVE_ONE,
    };

    LimitedLine {
        asset_id: asset_id.to_string(),
        amount: resolved * sign,
        quantity: quantity * sign,
        unit_price,
        is_below_minimum,
    }
}

/// Applies §4.6 in order: held quantity, turnover cap, available proceeds,
/// funding, rounding, minimum line size, remaining cash.
pub fn apply_limits(
    mut increases: Vec<DraftIncrease>,
    mut reductions: Vec<DraftReduction>,
    securities: &[SecurityInput],
    limits: &LimitsInput,
) -> LimitedAdjustments {
    cap_to_held_quantity(&mut reductions, securities);
    let reduction_factor = scale_to_turnover_cap(&mut reductions, limits.turnover_cap);

    // Step 3 — what the reductions actually raise, added to the cash the user
    // selected, is the funding available to the increases.
    let proceeds: Decimal = reductions.iter().map(|reduction| reduction.amount).sum();
    let available = limits.tracked_cash + limits.external_cash + proceeds;

    let increase_factor = scale_to_funding(&mut increases, available);

    let reductions: Vec<LimitedLine> = reductions
        .iter()
        .filter(|reduction| reduction.amount > Decimal::ZERO)
        .filter_map(|reduction| {
            let price = unit_price_of(securities, &reduction.asset_id)?;
            Some(finalize(
                &reduction.asset_id,
                reduction.amount,
                price,
                WorksheetDirection::Reduce,
                limits,
            ))
        })
        .collect();
    let increases: Vec<LimitedLine> = increases
        .iter()
        .filter(|increase| increase.amount > Decimal::ZERO)
        .filter_map(|increase| {
            let price = unit_price_of(securities, &increase.asset_id)?;
            Some(finalize(
                &increase.asset_id,
                increase.amount,
                price,
                WorksheetDirection::Increase,
                limits,
            ))
        })
        .collect();

    // Step 7 — whatever rounding and minimum-line reporting left behind. Not
    // redistributed: that would be another round of construction.
    let deployed: Decimal = increases.iter().map(|line| line.amount).sum();
    let raised: Decimal = reductions.iter().map(|line| line.amount.abs()).sum();
    let remaining_cash = limits.tracked_cash + limits.external_cash + raised - deployed;

    LimitedAdjustments {
        increases,
        reductions,
        scaling: AdjustmentScaling {
            reduction_factor,
            increase_factor,
        },
        remaining_cash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn category(id: &str, target_bps: i32, current_value: Decimal) -> CategoryTarget {
        CategoryTarget {
            category_id: id.to_string(),
            category_name: format!("{id} name"),
            target_bps,
            current_value,
        }
    }

    fn security(id: &str, values: &[(&str, Decimal)]) -> SecurityInput {
        SecurityInput {
            asset_id: id.to_string(),
            symbol: id.to_string(),
            unit_price: Some(dec!(100)),
            is_eligible_for_increase: true,
            category_values: values
                .iter()
                .map(|(category, value)| (category.to_string(), *value))
                .collect(),
            positions: vec![PositionInput {
                account_id: "acc-1".to_string(),
                quantity: dec!(10),
                can_reduce: true,
            }],
        }
    }

    #[test]
    fn gap_is_target_value_minus_current_value() {
        let categories = vec![
            category("EQUITY", 6000, dec!(5000)),
            category("FIXED_INCOME", 4000, dec!(5000)),
        ];

        let gaps = category_gaps(&categories, dec!(10000));

        assert_eq!(gaps[0], ("EQUITY".to_string(), dec!(1000)));
        assert_eq!(gaps[1], ("FIXED_INCOME".to_string(), dec!(-1000)));
    }

    #[test]
    fn gap_uses_planning_total_not_current_total() {
        // Taxonomy without a cash sleeve: the cash being deployed widens the
        // basis, so a category already at its target against the old total is
        // underweight against the new one.
        let categories = vec![category("EQUITY", 10000, dec!(10000))];

        let gaps = category_gaps(&categories, dec!(12000));

        assert_eq!(gaps[0].1, dec!(2000));
    }

    #[test]
    fn split_follows_current_holding_proportions() {
        let securities = vec![
            security("vti", &[("EQUITY", dec!(7500))]),
            security("vxus", &[("EQUITY", dec!(2500))]),
        ];

        let intents = split_gap("EQUITY", dec!(1000), &securities).unwrap();

        assert_eq!(intents.len(), 2);
        assert_eq!(intents[0].amount, dec!(750));
        assert_eq!(intents[1].amount, dec!(250));
    }

    #[test]
    fn split_of_a_negative_gap_produces_reductions() {
        let securities = vec![
            security("vti", &[("EQUITY", dec!(6000))]),
            security("vxus", &[("EQUITY", dec!(2000))]),
        ];

        let intents = split_gap("EQUITY", dec!(-800), &securities).unwrap();

        assert_eq!(intents[0].amount, dec!(-600));
        assert_eq!(intents[1].amount, dec!(-200));
    }

    #[test]
    fn split_weights_only_the_value_attributed_to_the_category() {
        // A 60/40 security carries 6000 of equity, not its whole 10000.
        let securities = vec![
            security(
                "blend",
                &[("EQUITY", dec!(6000)), ("FIXED_INCOME", dec!(4000))],
            ),
            security("vti", &[("EQUITY", dec!(2000))]),
        ];

        let intents = split_gap("EQUITY", dec!(800), &securities).unwrap();

        assert_eq!(intents[0].amount, dec!(600));
        assert_eq!(intents[1].amount, dec!(200));
    }

    #[test]
    fn combination_sums_every_category_intent_for_a_security() {
        let intents = vec![
            Intent {
                asset_id: "blend".to_string(),
                category_id: "EQUITY".to_string(),
                amount: dec!(600),
            },
            Intent {
                asset_id: "blend".to_string(),
                category_id: "FIXED_INCOME".to_string(),
                amount: dec!(-250),
            },
            Intent {
                asset_id: "vti".to_string(),
                category_id: "EQUITY".to_string(),
                amount: dec!(200),
            },
        ];

        let combined = combine_intents(&intents);

        assert_eq!(combined["blend"], dec!(350));
        assert_eq!(combined["vti"], dec!(200));
    }

    #[test]
    fn ineligible_securities_cannot_receive_an_increase() {
        let mut securities = vec![
            security("vti", &[("EQUITY", dec!(7500))]),
            security("vxus", &[("EQUITY", dec!(2500))]),
        ];
        securities[0].is_eligible_for_increase = false;

        let intents = split_gap("EQUITY", dec!(1000), &securities).unwrap();

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].asset_id, "vxus");
        assert_eq!(intents[0].amount, dec!(1000));
    }

    #[test]
    fn eligibility_never_restricts_reductions() {
        let mut securities = vec![security("vti", &[("EQUITY", dec!(8000))])];
        securities[0].is_eligible_for_increase = false;

        let intents = split_gap("EQUITY", dec!(-500), &securities).unwrap();

        assert_eq!(intents[0].asset_id, "vti");
        assert_eq!(intents[0].amount, dec!(-500));
    }

    #[test]
    fn protected_positions_cannot_carry_a_reduction() {
        let mut securities = vec![security("vti", &[("EQUITY", dec!(8000))])];
        securities[0].positions[0].can_reduce = false;

        let reason = split_gap("EQUITY", dec!(-500), &securities).unwrap_err();

        assert_eq!(reason, UnresolvedReason::NoEligibleSecurity);
    }

    #[test]
    fn category_with_nothing_recorded_is_unresolved() {
        let securities = vec![security("vti", &[("EQUITY", dec!(8000))])];

        let reason = split_gap("COMMODITIES", dec!(1000), &securities).unwrap_err();

        assert_eq!(reason, UnresolvedReason::NoRecordedSecurity);
    }

    #[test]
    fn category_with_everything_excluded_is_unresolved() {
        let mut securities = vec![security("gld", &[("COMMODITIES", dec!(1000))])];
        securities[0].is_eligible_for_increase = false;

        let reason = split_gap("COMMODITIES", dec!(1000), &securities).unwrap_err();

        assert_eq!(reason, UnresolvedReason::NoEligibleSecurity);
    }

    #[test]
    fn category_without_a_usable_price_is_unresolved() {
        let mut securities = vec![security("gld", &[("COMMODITIES", dec!(1000))])];
        securities[0].unit_price = None;

        let reason = split_gap("COMMODITIES", dec!(1000), &securities).unwrap_err();

        assert_eq!(reason, UnresolvedReason::NoUsablePrice);
    }

    #[test]
    fn carriers_worth_nothing_give_no_proportions_to_work_from() {
        let securities = vec![security("gld", &[("COMMODITIES", dec!(0))])];

        let reason = split_gap("COMMODITIES", dec!(1000), &securities).unwrap_err();

        assert_eq!(reason, UnresolvedReason::NoRecordedSecurity);
    }

    #[test]
    fn a_category_already_on_target_produces_nothing() {
        let securities = vec![security("vti", &[("EQUITY", dec!(8000))])];

        assert!(split_gap("EQUITY", Decimal::ZERO, &securities)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn spreading_keeps_placed_intents_and_unresolved_amounts_apart() {
        let categories = vec![
            category("EQUITY", 8000, dec!(7000)),
            category("COMMODITIES", 2000, dec!(1000)),
        ];
        let securities = vec![security("vti", &[("EQUITY", dec!(7000))])];

        let (intents, unresolved) = spread_gaps(&categories, dec!(10000), &securities);

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].asset_id, "vti");
        assert_eq!(intents[0].amount, dec!(1000));
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].category_id, "COMMODITIES");
        assert_eq!(unresolved[0].amount, dec!(1000));
        assert_eq!(unresolved[0].reason, UnresolvedReason::NoRecordedSecurity);
    }

    // ── Limits (§4.6) ────────────────────────────────────────────────────────

    fn limits() -> LimitsInput {
        LimitsInput {
            tracked_cash: Decimal::ZERO,
            external_cash: Decimal::ZERO,
            turnover_cap: None,
            min_line_amount: Decimal::ZERO,
            whole_shares_only: false,
        }
    }

    fn increase(asset_id: &str, amount: Decimal, funding: IncreaseFunding) -> DraftIncrease {
        DraftIncrease {
            asset_id: asset_id.to_string(),
            amount,
            funding,
        }
    }

    fn reduction(asset_id: &str, amount: Decimal) -> DraftReduction {
        DraftReduction {
            asset_id: asset_id.to_string(),
            amount,
        }
    }

    #[test]
    fn turnover_cap_derives_an_amount_from_basis_points() {
        assert_eq!(
            turnover_cap_value(dec!(10000), Some(1000)),
            Some(dec!(1000))
        );
        assert_eq!(turnover_cap_value(dec!(10000), None), None);
    }

    #[test]
    fn a_reduction_cannot_exceed_the_recorded_position() {
        // 10 units at 100 is 1000 held, against a 2500 intent.
        let securities = vec![security("vti", &[("EQUITY", dec!(1000))])];

        let result = apply_limits(
            vec![],
            vec![reduction("vti", dec!(2500))],
            &securities,
            &limits(),
        );

        assert_eq!(result.reductions[0].amount, dec!(-1000));
        assert_eq!(result.reductions[0].quantity, dec!(-10));
    }

    #[test]
    fn a_protected_position_holds_nothing_to_reduce() {
        let mut securities = vec![security("vti", &[("EQUITY", dec!(1000))])];
        securities[0].positions[0].can_reduce = false;

        let result = apply_limits(
            vec![],
            vec![reduction("vti", dec!(500))],
            &securities,
            &limits(),
        );

        assert!(result.reductions.is_empty());
    }

    #[test]
    fn turnover_cap_scales_every_reduction_by_the_same_factor() {
        let securities = vec![
            security("vti", &[("EQUITY", dec!(1000))]),
            security("vxus", &[("EQUITY", dec!(1000))]),
        ];
        let mut input = limits();
        input.turnover_cap = Some(dec!(600));

        let result = apply_limits(
            vec![],
            vec![reduction("vti", dec!(800)), reduction("vxus", dec!(400))],
            &securities,
            &input,
        );

        // 1200 of reductions scaled by 0.5 to fit a 600 cap.
        assert_eq!(result.scaling.reduction_factor, Some(dec!(0.5)));
        assert_eq!(result.reductions[0].amount, dec!(-400));
        assert_eq!(result.reductions[1].amount, dec!(-200));
    }

    #[test]
    fn turnover_cap_that_does_not_bind_is_not_reported() {
        let securities = vec![security("vti", &[("EQUITY", dec!(1000))])];
        let mut input = limits();
        input.turnover_cap = Some(dec!(900));

        let result = apply_limits(
            vec![],
            vec![reduction("vti", dec!(500))],
            &securities,
            &input,
        );

        assert_eq!(result.scaling.reduction_factor, None);
        assert_eq!(result.reductions[0].amount, dec!(-500));
    }

    #[test]
    fn increases_are_scaled_to_the_funding_available() {
        let securities = vec![security("vti", &[("EQUITY", dec!(1000))])];
        let mut input = limits();
        input.tracked_cash = dec!(400);

        let result = apply_limits(
            vec![increase("vti", dec!(1000), IncreaseFunding::Proceeds)],
            vec![],
            &securities,
            &input,
        );

        assert_eq!(result.scaling.increase_factor, Some(dec!(0.4)));
        assert_eq!(result.increases[0].amount, dec!(400));
    }

    #[test]
    fn a_shortfall_never_scales_increases_the_first_pass_already_funded() {
        // 500 of cash-funded increases, plus 500 that expected proceeds. The
        // turnover cap leaves only 200 of proceeds, so the 300 shortfall comes
        // off the proceeds-funded line alone.
        let securities = vec![
            security("vti", &[("EQUITY", dec!(1000))]),
            security("vxus", &[("EQUITY", dec!(1000))]),
            security("bnd", &[("FIXED_INCOME", dec!(1000))]),
        ];
        let mut input = limits();
        input.tracked_cash = dec!(500);
        input.turnover_cap = Some(dec!(200));

        let result = apply_limits(
            vec![
                increase("vti", dec!(500), IncreaseFunding::Cash),
                increase("vxus", dec!(500), IncreaseFunding::Proceeds),
            ],
            vec![reduction("bnd", dec!(500))],
            &securities,
            &input,
        );

        assert_eq!(result.scaling.increase_factor, Some(dec!(0.4)));
        assert_eq!(
            result.increases[0].amount,
            dec!(500),
            "cash-funded untouched"
        );
        assert_eq!(
            result.increases[1].amount,
            dec!(200),
            "absorbs the shortfall"
        );
    }

    #[test]
    fn funding_that_covers_everything_scales_nothing() {
        let securities = vec![security("vti", &[("EQUITY", dec!(1000))])];
        let mut input = limits();
        input.tracked_cash = dec!(1000);

        let result = apply_limits(
            vec![increase("vti", dec!(600), IncreaseFunding::Cash)],
            vec![],
            &securities,
            &input,
        );

        assert_eq!(result.scaling.increase_factor, None);
        assert_eq!(result.increases[0].amount, dec!(600));
        assert_eq!(result.remaining_cash, dec!(400));
    }

    #[test]
    fn whole_unit_policy_floors_quantities_and_leaves_the_residue_as_cash() {
        let securities = vec![security("vti", &[("EQUITY", dec!(1000))])];
        let mut input = limits();
        input.tracked_cash = dec!(950);
        input.whole_shares_only = true;

        let result = apply_limits(
            vec![increase("vti", dec!(950), IncreaseFunding::Cash)],
            vec![],
            &securities,
            &input,
        );

        // 9.5 units at 100 floors to 9, so 900 is deployed and 50 is left.
        assert_eq!(result.increases[0].quantity, dec!(9));
        assert_eq!(result.increases[0].amount, dec!(900));
        assert_eq!(result.remaining_cash, dec!(50));
    }

    #[test]
    fn a_sub_minimum_line_is_reported_rather_than_dropped() {
        let securities = vec![security("vti", &[("EQUITY", dec!(1000))])];
        let mut input = limits();
        input.tracked_cash = dec!(50);
        input.min_line_amount = dec!(200);

        let result = apply_limits(
            vec![increase("vti", dec!(50), IncreaseFunding::Cash)],
            vec![],
            &securities,
            &input,
        );

        assert_eq!(result.increases.len(), 1);
        assert!(result.increases[0].is_below_minimum);
        assert_eq!(result.increases[0].amount, dec!(50), "never re-rounded up");
    }

    #[test]
    fn reduction_proceeds_fund_increases() {
        let securities = vec![
            security("vti", &[("EQUITY", dec!(1000))]),
            security("bnd", &[("FIXED_INCOME", dec!(1000))]),
        ];

        let result = apply_limits(
            vec![increase("vti", dec!(700), IncreaseFunding::Proceeds)],
            vec![reduction("bnd", dec!(700))],
            &securities,
            &limits(),
        );

        assert_eq!(result.scaling.increase_factor, None);
        assert_eq!(result.increases[0].amount, dec!(700));
        assert_eq!(result.reductions[0].amount, dec!(-700));
        assert_eq!(result.remaining_cash, Decimal::ZERO);
    }
}
