ALTER TABLE daily_account_valuation ADD COLUMN cash_balance_base TEXT NOT NULL DEFAULT '0';
ALTER TABLE daily_account_valuation ADD COLUMN investment_market_value_base TEXT NOT NULL DEFAULT '0';
ALTER TABLE daily_account_valuation ADD COLUMN total_value_base TEXT NOT NULL DEFAULT '0';
ALTER TABLE daily_account_valuation ADD COLUMN cost_basis_base TEXT NOT NULL DEFAULT '0';
ALTER TABLE daily_account_valuation ADD COLUMN net_contribution_base TEXT NOT NULL DEFAULT '0';

-- Force recalculation so all rows get populated with real base values.
DELETE FROM daily_account_valuation;
