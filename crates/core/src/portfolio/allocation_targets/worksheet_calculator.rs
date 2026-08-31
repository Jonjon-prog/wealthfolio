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

use super::model::{UnresolvedCategoryAmount, UnresolvedReason, WorksheetDirection};

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
}
