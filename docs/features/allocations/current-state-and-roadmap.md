# Allocation & Rebalance — Current State, Research & Open Questions

Status: Living document Date: 2026-06-07 Audience: Afadil, contributors

---

## 1. Purpose

`v1-spec.md` is the original reference Afadil authored and is intentionally
**kept frozen** as the historical scope document. This document is its living
companion. It records:

- What actually shipped (and how it diverged from the v1 spec).
- Industry research on how comparable tools approach rebalancing.
- Improvement opportunities backed by that research.
- Open questions we need Afadil to answer to plan v2 / the rest of the SOTA
  north-star.

The goal is to converge on a correct, agreed picture of where the feature stands
and where it goes next — including whether a **v2 design document** already
exists on Afadil's side.

---

## 2. What shipped (vs `v1-spec.md` milestones)

Verified against `main` on 2026-06-07.

| Milestone                | PR / commit          | Status | Notes                                                                          |
| ------------------------ | -------------------- | ------ | ------------------------------------------------------------------------------ |
| **M1** Targets & Drift   | #1025                | ✅     | scope targets, weights (bps), drift, triggers `manual`/`threshold`             |
| **M2** Cash-Flow Planner | #1036, #1044, #1048  | ✅     | cash auto-read, suggested buys, CSV, `min_trade`/`whole_shares`/`rebalance_to` |
| ↳ Drift planner (PR-B)   | #1048                | ✅     | `DriftPriorityOptimizer` — exposure-aware greedy (multi-category ETF)          |
| **M3** Simple Sell Mode  | #1054                | ✅     | `ScenarioMode` {CashFlowOnly, SellToRebalance, Hybrid}, `allow_sells`          |
| ↳ Planner redesign       | ae4f626a0, 2952d794e | ✅     | UI redesign, settings in one card, taxonomy wired into drift, react-query      |
| ↳ Proportional top-up    | 31a12d74d, bb765c502 | ✅     | deploys leftover cash ∝ `target_bps` after greedy drift gains exhaust          |
| **M4** Integration Hooks | —                    | ❌     | tax-lot trait boundary + no-op tax warning provider — **not started**          |

So v1 (M1–M3) is delivered. **M4 is the only unshipped v1 milestone**, and it is
a thin extension-point milestone (no tax logic, just a trait boundary).

---

## 3. Intentional divergences from `v1-spec.md`

The frozen spec describes a few things we deliberately changed during
implementation. They are listed here so the spec and reality don't silently
disagree.

| Spec says                                      | Reality                                     | Why                                                                                                                                                                                 |
| ---------------------------------------------- | ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `saveRebalanceDraft`, `rebalance_drafts` table | **Removed** — settled in PR #1036           | Afadil's call ("remove saved rebalance draft storage for now"); a plan is stale once prices move. Future history, if wanted, is `RebalanceRun` (SOTA §5.8), not `rebalance_drafts`. |
| `base_currency` per target                     | **Deferred** — app-level base currency only | No per-profile FX picker yet; revisit when multi-currency targets are needed                                                                                                        |
| Drift band = absolute `drift_band_bps`         | Absolute today; moving to hybrid            | §4.3 — agreed to relative ~20% + absolute floor; absolute was the simple v1 default                                                                                                 |
| (not in spec) proportional top-up              | **Added** — see `rebalance-algorithm.md`    | Deploys residual cash once drift can't be improved, instead of leaving it idle                                                                                                      |

---

## 4. Industry research — how comparable tools rebalance

Research conducted 2026-06-07 to ground the roadmap discussion. Full sources in
§7.

### 4.1 Cash-flow-first, sell-second is the standard

Betterment, Wealthfront and M1 Finance all rebalance **reactively with cash
flows first** (route deposits/dividends to underweight sleeves, withdrawals from
overweight) and only **proactively sell** when cash flow can't keep drift within
band. This is exactly our `CashFlowOnly → Hybrid → SellToRebalance` model. **Our
three scenario modes match industry practice.**

This is not just robo marketing: the **SEC / Investor.gov** beginners' guide
lists the _same three_ rebalancing methods — (1) sell over-weight & buy
under-weight, (2) buy under-weight only, (3) route new contributions to
under-weight categories — so our mode set matches the US regulator's own
investor guidance, not only commercial tools.

### 4.2 Threshold beats calendar rebalancing

A Vanguard study of a 60/40 portfolio since 1926 found monthly/quarterly
calendar rebalancing produced **no risk/return improvement over annual — it just
raised turnover and cost**. Threshold (tolerance-band) rebalancing only trades
when drift breaches a band. **We already ship threshold triggers** (and monitor,
not auto-trade — same posture as iRebal).

Institutional corroboration: **Vanguard's Dec-2024 research** recommends a ~200
bps trigger with a 175 bps destination (i.e. rebalance toward the band, not the
exact target), and **JPMorgan Asset Management** (Jan-2025) argues for a
threshold-driven approach over fixed calendars. SEC/Investor.gov presents
threshold rebalancing alongside calendar as an accepted method.

### 4.3 Relative vs. absolute bands — multi-study view

Reviewed across five sources to test whether the field agrees.

**Where the studies converge:**

- **Absolute bands are structurally flawed for multi-class portfolios.** A fixed
  ±5pp band under-corrects small sleeves and over-corrects large ones (Kitces,
  Portfolio Construction Forum, BBH). A 5% target sleeve under a ±5pp band can
  double to 10% or vanish to 0% and still read "in band." Relative bands scale
  tolerance to each sleeve's weight, so the consensus is that **relative is the
  fairer trigger**. Tamarac's default guardrail is ±20% relative.
- **Rebalance to the band edge, not the target.** Leland / MIT working papers
  show trading back only to the nearest edge of the no-trade region cuts
  transaction costs by ~50% vs. trading to the exact target. This is already our
  `rebalance_goal: nearest_band` vs. `exact_target` — we match the academic
  result.

**Where the studies diverge:**

- **The optimal width is not universal.** Daryanani (2008, _Journal of Financial
  Planning_) found ±20% relative optimal (10–15% too tight, 25% too loose).
  Vanguard's 2024 target-date research uses ~200 bps **absolute** at the
  major-asset level. A common hybrid (via Kitces) is **±25% relative at
  sub-class level + ±5% absolute at major-asset level**. Leland shows the true
  optimum is a _function_ of transaction cost, volatility, correlation and the
  target mix — no single number.
- **Whether rebalancing adds return is disputed.** Daryanani claimed +0.45%/yr
  (doubling the benefit vs. annual). Marlena Lee (2008) rebutted that the return
  gain is **not statistically different from zero** and is sample-dependent.
  Vanguard agrees the return lift is "slight" — the real, reliable benefit is
  **risk control + lower turnover/cost**, not alpha.

**Decision (agreed 2026-06-07):** move to a **hybrid band — relative ~20% by
default with an absolute floor**. The absolute floor solves the zero/near-zero
target edge case (a 0%-target sleeve has no meaningful relative band) and the
"tiny sleeve, micro-trades" case. We will frame the change as _fairer trigger +
lower churn_, not as a return enhancement (the evidence for alpha is weak). See
§5 item 1.

### 4.4 Drift definition convention — non-issue for us

Betterment reports portfolio drift as **total absolute deviation ÷ 2** (one
sleeve 5pp over + another 5pp under = 5% drift, not 10%, avoiding
double-counting the same displaced money). We internally sum `Σ|drift_bps|`, but
**only as the optimizer's objective** — where the ÷2 is a constant factor that
does not change which trade scores best. The **displayed** headline is **"max
drift"** (`maxDriftBps`, the single worst sleeve), not a summed total, so it has
no double-counting problem. **No change needed** — recorded only to show we
checked.

### 4.5 Asset location / household optimization (big future lever)

Vanguard, Fidelity, Morningstar and Bogleheads all document **asset location**:
hold tax-inefficient assets (bonds, REITs) in tax-deferred accounts, high-growth
(stocks) in Roth/taxable. Rebalancing then happens **across accounts** (route
contributions between accounts) rather than selling within one. This is the SOTA
"household / account-group + account placement" theme (Phase 3) and the single
largest differentiator vs. the open-source field.

**Caveat: the US literature is US-tax-specific** (401k/IRA/Roth). Hard-coding
those categories would be wrong for a worldwide app. The generalized model that
covers every jurisdiction is the **account tax-wrapper** approach in §4.9, which
we believe is the better foundation for this lever.

### 4.6 Tax-aware (Phase 4)

Wealthfront uses a **cost-benefit** model per lot (trading cost vs. opportunity
cost of waiting). Betterment uses **Parallel Position Management** (paired
securities to harvest losses without triggering wash sales, routing every
deposit/withdrawal tax-efficiently). This is the Phase 4 tax-lot work and
depends on lot-level cost basis we don't persist yet.

### 4.7 Value averaging (Phase 3 funding)

Value averaging = set a target portfolio value per period, contribute (or sell)
whatever closes the gap. It's literally "DCA + rebalancing." Maps directly to
the SOTA `FundingPolicy` + recurring-contribution concept.

### 4.8 Where the open-source field is

Ghostfolio and Portfolio Performance offer **basic single-dimension
rebalancing** (drift vs. a flat target list). **Wealthfolio is already ahead**
with exposure-aware multi-category planning (one ETF updates several taxonomy
sleeves per trade). The differentiators that would extend the lead are §4.3
(relative bands), §4.9 (account tax-wrapper) and §4.6 (tax-aware).

### 4.9 Account tax-wrapper model (worldwide) — proposed

The asset-location lever (§4.5) only works if the app knows how each account is
taxed. Rather than hard-coding US categories, model a **tax wrapper per
account**; assets inherit their account's treatment. This was validated against
account types across many jurisdictions — under the local names there are only a
few **universal archetypes**:

| Archetype        | Behaviour                                      | Examples                                          |
| ---------------- | ---------------------------------------------- | ------------------------------------------------- |
| **Taxable**      | Taxed on income/gains as incurred              | US brokerage, FR CTO, UK GIA                      |
| **Tax-deferred** | Pre-tax in, taxed on withdrawal                | US 401k/IRA, CA RRSP, UK SIPP, AU Super, FR PER   |
| **Tax-free**     | After-tax in, growth & withdrawal exempt       | US Roth, CA TFSA, UK ISA, JP NISA, FR PEA (>5y)   |
| **Conditional**  | Treatment changes after a holding-period bound | FR assurance-vie (8y), FR PEA (5y), some DE funds |

So the model is **a small enum of archetypes + a per-jurisdiction catalog** that
maps a local wrapper name to an archetype plus parameters (holding-period
threshold, flat rate, allowance). The catalog is the only country-specific part;
the engine is universal.

**Validated against official tax authorities.** The archetypes hold up against
primary government sources, which also give the exact catalog parameters:

- 🇫🇷 **impots.gouv.fr / economie.gouv.fr**: assurance-vie reduced rate (7.5%) +
  annual allowance **after 8 years**; PEA income-tax exemption **after 5 years**
  (social levies remain); CTO under the 30% PFU. → confirms the **Conditional**
  archetype with concrete bounds (96 / 60 months).
- 🇺🇸 **IRS**: Roth qualified distribution tax-free after 5y & 59½ (tax-free);
  traditional 401k/IRA pre-tax (tax-deferred); tax lots are FIFO or specific-ID,
  long-term > 1 year. → confirms tax-free / tax-deferred + the Q9 boundary.
- 🇬🇧 **gov.uk (HMRC)**: ISA income/gains/dividends tax-free; SIPP/pension
  exempt. 🇨🇦 **canada.ca (CRA)**: TFSA tax-free, RRSP withdrawal taxable.

**One honest refinement (🇦🇺 ATO):** Australian super taxes _internal_ earnings
at 15% (concessional), so it is not pure deferral. The model should allow an
**optional internal earnings rate** on a wrapper, or document super as an
approximation — flagged so we don't over-simplify.

**What the planner does with it (no tax engine required):**

- **Sell ordering** — prefer selling in tax-free / tax-deferred accounts (no
  taxable event) before taxable accounts.
- **Warnings** — selling in a Taxable wrapper → "may realize capital gains (not
  estimated)"; selling in a Conditional wrapper before its bound → "selling
  before {date} loses the {benefit}".
- **Cash routing** — when buying an underweight sleeve, suggest the account
  where it is most tax-efficient to hold (the asset-location placement
  decision).
- **Holding-period surfacing** — show "T-minus N until tax-free" per account /
  holding. Few tools do this; US tools ignore it because US wrappers have no
  such condition. Genuine differentiator for FR/DE/etc.

**Two scope levels (a key question for Afadil):**

1. **Lightweight (proposed MVP).** Account gets a `tax_treatment` archetype tag
   (+ optional holding-period bound). Planner uses it for sell-ordering, cash
   routing and warnings only. **No gain computation, no tax lots** — decoupled
   from Phase 4. Captures ~80% of the value cheaply.
2. **Full.** Real tax engine: cost basis, short/long gains, dated allowances per
   country. Large surface; depends on lot-level data (Phase 4).

**Data shape (mini-schema, lightweight level).** The archetype lives on the
account; the catalog is reference data (seed or addon).

```sql
-- on the existing accounts table
ALTER TABLE accounts
  ADD COLUMN tax_treatment TEXT NOT NULL DEFAULT 'taxable'
      CHECK (tax_treatment IN ('taxable','tax_deferred','tax_free','conditional'));
ADD COLUMN tax_wrapper_id TEXT;       -- FK to catalog, display + params (nullable)
ADD COLUMN tax_free_after TEXT;       -- ISO date: when a 'conditional' wrapper turns favourable
ADD COLUMN avoid_selling INTEGER NOT NULL DEFAULT 0;  -- ultra-light bridge (account DNT)

-- per-jurisdiction catalog (seedable, or shipped by an addon)
CREATE TABLE tax_wrappers (
    id         TEXT PRIMARY KEY,       -- 'fr_assurance_vie', 'us_roth_ira', 'uk_isa'
    country    TEXT NOT NULL,          -- 'FR', 'US', 'UK', ...
    label      TEXT NOT NULL,          -- 'Assurance-vie', 'Roth IRA', 'ISA'
    archetype  TEXT NOT NULL,          -- maps to tax_treatment
    holding_period_months INTEGER,     -- e.g. 96 for assurance-vie (8y); null if none
    internal_earnings_rate_bps INTEGER, -- e.g. 1500 for AU super (15%); null = pure deferral
    notes      TEXT
);
```

The planner reads only `tax_treatment` (+ `tax_free_after`, `avoid_selling`) —
no join required at plan time. Picking a `tax_wrapper_id` in the UI pre-fills
the archetype and holding period from the catalog. Core stays
jurisdiction-agnostic; the catalog is the only country-specific surface and can
be an addon.

**Adjacent ideas:**

- **Catalog as an addon** — ship the per-jurisdiction wrapper catalog through
  the addon SDK (SOTA Phase 5). The community maintains country rules; core
  stays jurisdiction-agnostic. Strong fit for a worldwide local-first app.
- **Ultra-light bridge** — even before any tax modelling, an `avoid_selling`
  flag per account (account-level do-not-trade) gives most of the protection
  with near-zero modelling.

---

## 5. Improvement opportunities (research-backed, no commitment)

Ordered by leverage-to-effort.

1. **Hybrid tolerance bands** (§4.3) — **agreed direction.** Move to relative
   ~20% by default with an absolute floor (floor handles zero-target sleeves and
   micro-trades). Per-target setting, absolute override allowed. Low effort,
   improves trigger fairness; framed as fairness/churn, not alpha.
2. **M4 tax-lot trait boundary** — cheap to add now, unblocks Phase 4 later
   without schema churn. Already specified in `v1-spec.md` §6.5.
3. **Turnover cap + do-not-trade list** (SOTA Phase 2) — standard advisor
   controls; both are simple planner constraints layered on the existing greedy.
   The do-not-trade list at **account** granularity is also the ultra-light
   bridge toward §4.9.
4. **Account tax-wrapper model — lightweight** (§4.9, SOTA Phase 3) — the
   highest differentiation lever. MVP = per-account archetype tag +
   sell-ordering + cash routing + warnings, no tax engine. Needs the account
   metadata + (eventually) household grouping.
5. **Value-averaging funding policy** (§4.7, SOTA Phase 3).
6. **Tax-aware optimizer — full** (§4.6/§4.9 level 2, SOTA Phase 4) — gated on
   lot-level cost basis.
7. **Drift-number convention** (§4.4) — non-issue, no action (kept for the
   record).

### 5.1 Proposed change: absolute → hybrid tolerance bands

This is the one change we want to **commit to and implement next** (pending your
sign-off on the numbers, Q5). Writing it out in full so the intent is explicit.

**Problem.** Today a target carries a single absolute band, `drift_band_bps`
(e.g. ±500 bps = ±5pp), applied identically to every sleeve. The breach test is:

```
out_of_band  ⇔  |current_bps − target_bps| > drift_band_bps
```

A fixed ±5pp band is too tight on large sleeves and far too loose on small ones:
a 5%-target sleeve can run to 10% (double) or 0% (gone) and still read "in
band." The literature is consistent that absolute bands under-correct small
sleeves and over-correct large ones (§4.3).

**Proposal — hybrid band (relative with an absolute floor).** Each sleeve's
allowed deviation becomes the **larger** of a relative term and an absolute
floor:

```
band_c   = max(target_bps_c × relative_factor, absolute_floor_bps)
out_of_band_c  ⇔  |current_bps_c − target_bps_c| > band_c
```

- `relative_factor` default **0.20** (Daryanani / Tamarac), per-target editable.
- `absolute_floor_bps` default small (e.g. **100 bps = 1pp**), per-target
  editable. The floor is what makes the rule sane for **zero / near-zero target
  sleeves** (a 0% target has no meaningful relative band) and stops micro-trades
  on tiny sleeves.

Example, `relative_factor = 0.20`, `floor = 100 bps`:

| Sleeve target | Relative term | Floor   | Effective band | In-band range |
| ------------- | ------------- | ------- | -------------- | ------------- |
| 50%           | ±1000 bps     | 100 bps | ±1000 bps      | 40–60%        |
| 10%           | ±200 bps      | 100 bps | ±200 bps       | 8–12%         |
| 1%            | ±20 bps       | 100 bps | ±100 bps       | 0–2%          |
| 0%            | ±0 bps        | 100 bps | ±100 bps       | 0–1%          |

**Scope of the change.**

- Add a band mode to the target: `band_type: absolute | hybrid` (+ store
  `relative_factor`, keep `drift_band_bps` as the absolute floor). Default new
  targets to `hybrid`; existing targets stay `absolute` until edited
  (back-compatible, no silent behaviour change).
- The breach test moves from one global band to a **per-sleeve** band in
  `DriftService` and wherever `out_of_band` / `maxDrift` status is computed.
- Planner goal thresholds (`NearestBand`) already operate per category; they
  switch to the per-sleeve `band_c`.
- UI: target editor gains a band-mode toggle and the two values; drift rows show
  each sleeve's own band.

**Data shape (mini-schema).** Two new columns on the target; the existing
`drift_band_bps` is reused as the absolute floor.

```sql
ALTER TABLE allocation_targets
  ADD COLUMN band_type TEXT NOT NULL DEFAULT 'absolute'
      CHECK (band_type IN ('absolute', 'hybrid'));
ADD COLUMN relative_factor_bps INTEGER NOT NULL DEFAULT 2000;  -- 2000 = 20%
-- drift_band_bps keeps its meaning:
--   band_type='absolute' → the band itself
--   band_type='hybrid'   → the absolute floor
```

```rust
// per-sleeve effective band, computed in DriftService
fn sleeve_band_bps(t: &AllocationTarget, target_bps: i32) -> i32 {
    match t.band_type {
        BandType::Absolute => t.drift_band_bps,
        BandType::Hybrid => {
            let relative = target_bps * t.relative_factor_bps / 10_000;
            relative.max(t.drift_band_bps) // absolute floor
        }
    }
}
// breach: |current_bps - target_bps| > sleeve_band_bps(target, target_bps)
```

Existing targets keep `band_type='absolute'` → identical behaviour until a user
opts in. New targets default to `hybrid`.

**Framing (important).** We will present this as a **fairer trigger + less
churn**, _not_ as a return enhancement — the evidence that rebalancing adds
alpha is contested (Lee 2008 vs. Daryanani; Vanguard calls it "slight"). Honest
copy avoids over-claiming.

**Effort.** Low/medium. Mostly the per-sleeve band computation + two new
fields + editor UI. No optimizer rewrite (the greedy already scores per-category
drift).

**Status:** proposed, ready to build once Q5 (default factor + floor) is
confirmed.

---

## 6. Open questions for Afadil

Each item carries a one-line **Research** finding (how the field handles it) and
the **lean** we'd recommend, so these are decisions to confirm — not blank
questions.

### 6.0 Already settled (recorded, not questions)

- **Drafts — settled (PR #1036).** Saved rebalance drafts were removed on
  Afadil's recommendation; the planner is stateless. Future history, if ever
  wanted, is `RebalanceRun` (SOTA §5.8), not `rebalance_drafts`. No action.
- **Drift ÷2 convention — non-issue** (§4.4). No action.
- **Absolute → hybrid bands — direction agreed** (§5.1 proposal). Pending only
  Afadil's sign-off on defaults (Q5).

### 6.1 Immediate (closing out v1)

- **Q1. M4 hooks** — implement the tax-lot trait boundary + no-op tax warning
  provider now (closes v1), or defer until the tax feature actually starts?
- **Q3. base_currency per target** — reintroduce, or stay app-level for v2?

### 6.2 SOTA Phase 2 remainder

- **Q5. Hybrid drift bands** — see the §5.1 proposal. Pending only your sign-off
  on the default relative % and the absolute floor value.
- **Q4. Turnover cap** and **do-not-trade list** — in scope for the next
  iteration?
  - **Research:** advisor tools express turnover control two ways — a simple **%
    turnover cap** per rebalance (churn/cost control, no tax data), and a richer
    **gain/loss budget** (annual $ or % cap on realised capital gains, used as
    an optimizer constraint — but that needs tax lots). Do-not-trade lists are
    standard: mark positions the rebalancer must not sell (low basis,
    concentrated/restricted, employer stock).
  - **Lean:** ship the **non-tax pair now** — a per-plan turnover cap (% of
    portfolio) + an asset-level "do not sell" flag (and the account-level
    `avoid_selling` from §4.9). Defer the gain/loss budget to Phase 4 (needs
    lots).

### 6.3 SOTA Phase 3 (advanced policies)

- **Q6.** `FundingPolicy` / recurring contributions / **value averaging** —
  priority and intended UX?
  - **Research:** robo/broker auto-invest = a schedule (weekly/biweekly/monthly)
    - amount + source account; each contribution is routed through cash-flow
      rebalancing (which we already have). Value averaging differs by targeting
      a portfolio **value** trajectory and contributing/selling the gap, not a
      fixed amount.
  - **Lean:** a `FundingPolicy` = schedule + rule (fixed amount **or** value
    target). Because we're local-first with no broker, it **generates a
    suggested plan on schedule and notifies** — it does not auto-execute. Reuses
    the existing cash-flow planner; value averaging is one rule variant.
- **Q7.** `TargetGuardrail` enforcement (vs. monitor-only today) — what does
  "enforced" mean operationally without execution?
  - **Research:** the field splits guardrails into **monitor-only** (alert when
    out of band, human acts — non-discretionary) vs. **enforced**
    (auto-rebalance on breach — discretionary robo). Enforcement assumes
    execution authority.
  - **Lean:** for a local-first app with no broker link, "enforced" cannot mean
    auto-trade. Define it as a **persisted, actively-notifying rule** (badge /
    notification on breach, persists across sessions) vs. today's passive band
    you only see on the page. Worth confirming this interpretation with Afadil.
- **Q8.** **Account tax-wrapper model** (§4.9) — is the per-account archetype +
  asset-location placement the intended Phase 3 centerpiece? Which scope:
  **lightweight** (tag + warnings + routing, no tax engine) or **full** (tax
  engine, depends on lots)? We recommend starting lightweight.
- **Q8b.** Should the **per-jurisdiction wrapper catalog be an addon**
  (community-maintained) rather than core? Fits the addon SDK and keeps core
  jurisdiction-agnostic.

### 6.4 SOTA Phase 4–5

- **Q9.** Tax-lot model — does this wait for a separate cost-basis/lot feature,
  and should allocation just expose the trait (Q1)?
  - **Research:** the IRS recognises only **FIFO** and **specific
    identification**; HIFO/min-tax/LIFO are flavours of specific ID (LIFO is not
    allowed for securities). Schwab's "Tax Lot Optimizer" = automated specific
    ID over basis + holding period. This is its own cost-basis subsystem.
  - **Lean:** keep tax lots **out of allocation**. Allocation exposes the M4
    trait (Q1) and consumes lot data when a separate cost-basis feature provides
    it. Confirms our boundary.
- **Q10.** `HoldingTarget` (per-ticker targets, SOTA §5.4) — v2 priority, or
  stays taxonomy-only?
  - **Research:** M1's "pies" are the reference — per-holding target %, must sum
    to 100%, min 1% per slice, nestable sub-pies; every deposit/rebalance flows
    to per-holding targets. It's an alternative axis to taxonomy targets ("10%
    VTI" vs. "30% equities").
  - **Lean:** real v2 feature, not a quick add. Our exposure-aware engine
    already works per holding, so the planner side is tractable; the work is the
    data model + editor + sum/lock rules. Sequence it **after** the band +
    tax-wrapper work.
- **Q11.** Phase 5 model marketplace / addon SDK for allocation state — on the
  roadmap or out of scope?
  - **Research:** institutional "model marketplaces" (Betterment, Altruist,
    IBKR, OneVest) are curated catalogs of provider portfolios — heavy, advisor-
    facing. The realistic open-source analogue is **import/export of target
    models as files** (Morningstar-style holdings import) plus community-shared
    templates.
  - **Lean:** for a local-first app, do **import/export of a target model**
    (JSON) + optional **addon-bundled template packs**. No hosted marketplace.
    Fits the addon SDK (same vehicle as the §4.9 wrapper catalog).

### 6.5 Direction

- **Q12. Do you already have a v2 / SOTA-continuation design document?** If so
  we align to it. If not, we can draft one from this research and the SOTA spec
  for your review.
- **Q13.** What's the next milestone you want prioritized: finish Phase 2
  (hybrid bands → turnover/DNT), or jump to Phase 3 (account tax-wrapper /
  funding / guardrails)? Our lean: **hybrid bands first** (small, agreed), then
  the **lightweight account tax-wrapper** (biggest differentiation).

---

## 7. References

Research sources (accessed 2026-06-07):

- Betterment —
  [rebalancing methods](https://www.betterment.com/help/portfolio-rebalancing-methods),
  [tax-loss harvesting methodology](https://www.betterment.com/resources/tax-loss-harvesting-methodology)
- Wealthfront —
  [investment methodology](https://research.wealthfront.com/whitepapers/investment-methodology/),
  [tax-loss harvesting](https://research.wealthfront.com/whitepapers/tax-loss-harvesting/)
- M1 Finance —
  [dynamic rebalancing](https://help.m1.com/en/articles/9332105-how-to-rebalance-your-m1-investment-account)
- Kitces —
  [time horizons vs. tolerance bands](https://www.kitces.com/blog/best-opportunistic-rebalancing-frequency-time-horizons-vs-tolerance-band-thresholds/)
- Daryanani (2008), _Journal of Financial Planning_ —
  [Opportunistic Rebalancing](https://www.financialplanningassociation.org/article/journal/JAN08-opportunistic-rebalancing-new-paradigm-wealth-managers)
- Marlena Lee (2008) — rebuttal on return-significance (via Bogleheads /
  Portfolio Construction Forum)
- Portfolio Construction Forum —
  [time horizons vs. tolerance bands (PDF)](https://obj.portfolioconstructionforum.edu.au/articles_perspectives/PorfolioConstruction-Forum_Finding-the-optimal-rebalancing-frequency-time-horizons-vs-tolerance-bands.pdf)
- BBH —
  [rebalancing for taxable investors](https://www.bbh.com/us/en/insights/capital-partners-insights/our-approach-to-portfolio-rebalancing-for-taxable-investors.html)
- Leland / MIT —
  [optimal rebalancing & no-trade region (PDF)](https://web.mit.edu/albota/www/SSRN-id639284.pdf)
- Vanguard —
  [The Rebalancing Edge, 2024 (PDF)](https://corporate.vanguard.com/content/dam/corp/research/pdf/the_rebalancing_edge_optimizing_target_date_fund_rebalancing_through_threshold_based_strategies.pdf),
  [asset location](https://investor.vanguard.com/investor-resources-education/article/asset-location-can-lead-to-lower-taxes)
- Tamarac —
  [Advisor Rebalancing](https://www.tamaracinc.com/_Media2014/PDF/Tamarac-Advisor-Rebalancing.pdf)
- Bogleheads —
  [tax-efficient fund placement](https://www.bogleheads.org/wiki/Tax-efficient_fund_placement),
  [value averaging](https://www.bogleheads.org/wiki/Value_averaging)
- Account types by jurisdiction — Sun Life
  [TFSA/RRSP/FHSA](https://www.sunlifeglobalinvestments.com/en/insights/investor-education/getting-started/comparison-tax-advantaged-savings-accounts-tfsa-rrsp-fhsa/);
  DayTrading.com
  [retirement plans by country](https://www.daytrading.com/retirement-plans-by-country)
- Ghostfolio — [repo](https://github.com/ghostfolio/ghostfolio); Portfolio
  Performance — [site](https://www.portfolio-performance.info/en/)

Roadmap-item research (§6):

- Turnover / gain budget — SS&C Black Diamond
  [tax-aware rebalancing](https://www.sscblackdiamond.com/discover/blog/tax-aware-rebalancing-the-missing-link-between-planning-and-portfolio/),
  Red-Black
  [tax-smart rebalancing](https://redblacksoftware.com/insights/whitepapers/tax-smart-portfolio-rebalancing)
- Funding / recurring — Vanguard
  [recurring investing](https://investor.vanguard.com/investor-resources-education/portfolio-management/making-regular-investments)
- Guardrails monitor vs. enforced — WE Family Offices
  [discretionary vs. non-discretionary](https://www.wefamilyoffices.com/resource/what-is-the-difference-between-discretionary-and-non-discretionary-investment-management/)
- Tax lots — Charles Schwab
  [know your cost basis](https://www.schwab.com/learn/story/save-on-taxes-know-your-cost-basis)
- Per-holding targets — M1
  [custom pies](https://help.m1.com/en/articles/9332122-creating-and-adding-custom-pies-to-your-m1-portfolio)
- Model marketplace — Altruist
  [model marketplace](https://altruist.com/model-marketplace/)

Institutional & government sources (primary, used to cross-check the above):

- Rebalancing methods — SEC / Investor.gov
  [beginners' guide to asset allocation, diversification & rebalancing](https://www.investor.gov/additional-resources/general-resources/publications-research/info-sheets/beginners-guide-asset);
  FINRA
  [rebalancing your portfolio](https://www.finra.org/investors/learn-to-invest/key-investing-concepts/rebalancing-your-portfolio)
- Rebalancing thresholds — Vanguard
  [The Rebalancing Edge, Dec 2024 (PDF)](https://corporate.vanguard.com/content/dam/corp/research/pdf/the_rebalancing_edge_optimizing_target_date_fund_rebalancing_through_threshold_based_strategies.pdf);
  J.P. Morgan Asset Management
  [rebalancing strategy, Jan 2025 (PDF)](https://am.jpmorgan.com/content/dam/jpm-am-aem/americas/us/en/insights/portfolio-insights/rebalancing-strategy-after-an-unusual-year-a-thoughtful-approach-is-needed.pdf)
- 🇺🇸 IRS —
  [traditional & Roth IRAs](https://www.irs.gov/retirement-plans/traditional-and-roth-iras),
  [designated Roth accounts](https://www.irs.gov/retirement-plans/plan-participant-employee/retirement-topics-designated-roth-account);
  Investor.gov
  [tax-advantaged accounts](https://www.investor.gov/introduction-investing/investing-basics/investment-accounts/tax-advantaged-accounts)
- 🇫🇷 impots.gouv.fr —
  [assurance-vie & PEA](https://www.impots.gouv.fr/particulier/lassurance-vie-et-le-pea-0);
  economie.gouv.fr
  [PFU](https://www.economie.gouv.fr/particuliers/impots-et-fiscalite/gerer-mes-autres-impots-et-taxes/comment-fonctionne-le-prelevement)
- 🇬🇧 gov.uk —
  [how ISAs work](https://www.gov.uk/individual-savings-accounts/how-isas-work)
- 🇨🇦 canada.ca —
  [about the TFSA](https://www.canada.ca/en/revenue-agency/services/tax/individuals/topics/tax-free-savings-account/what.html)
- 🇦🇺 ATO —
  [tax and super](https://moneysmart.gov.au/how-super-works/tax-and-super)

Internal:

- `docs/features/allocations/v1-spec.md` (frozen scope reference)
- `docs/features/allocations/sota-target-model-spec.md` (north-star)
- `docs/features/allocations/rebalance-algorithm.md` (current algorithm)
