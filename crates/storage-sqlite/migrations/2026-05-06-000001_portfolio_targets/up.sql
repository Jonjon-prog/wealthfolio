-- Portfolio target allocation profiles
CREATE TABLE IF NOT EXISTS portfolio_targets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    account_id TEXT NOT NULL,
    taxonomy_id TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    rebalance_mode TEXT NOT NULL DEFAULT 'buy_only',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (taxonomy_id) REFERENCES taxonomies(id)
);

CREATE INDEX IF NOT EXISTS idx_portfolio_targets_account ON portfolio_targets(account_id);

-- Category-level target allocations within a profile
CREATE TABLE IF NOT EXISTS portfolio_target_allocations (
    id TEXT PRIMARY KEY NOT NULL,
    target_id TEXT NOT NULL,
    category_id TEXT NOT NULL,
    target_percent INTEGER NOT NULL,
    is_locked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (target_id) REFERENCES portfolio_targets(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_target_allocations_target ON portfolio_target_allocations(target_id);

-- Per-holding allocation targets within a category allocation
CREATE TABLE IF NOT EXISTS holding_targets (
    id TEXT PRIMARY KEY NOT NULL,
    allocation_id TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    target_percent INTEGER NOT NULL,
    is_locked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (allocation_id) REFERENCES portfolio_target_allocations(id) ON DELETE CASCADE,
    FOREIGN KEY (asset_id) REFERENCES assets(id) ON DELETE CASCADE,
    UNIQUE(allocation_id, asset_id)
);

CREATE INDEX IF NOT EXISTS idx_holding_targets_allocation_id ON holding_targets(allocation_id);
CREATE INDEX IF NOT EXISTS idx_holding_targets_asset_id ON holding_targets(asset_id);
