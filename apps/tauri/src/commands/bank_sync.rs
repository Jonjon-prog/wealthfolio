//! Tauri commands for local bank sync via Woob (https://woob.tech).
//!
//! Woob is an open-source Python library that scrapes French bank websites
//! directly — no API key, no subscription, no cloud account required.
//!
//! Prerequisites (user side):
//!   pip3 install woob
//!   woob bank backends add boursorama   (or any other supported bank)
//!
//! Credentials are managed entirely by Woob locally (~/.config/woob/).
//! This module never touches credentials — it only calls `woob bank` commands.

use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::process::Command;
use wealthfolio_connect::broker::AccountUniversalActivityCurrency;

use crate::context::ServiceContext;

// ─────────────────────────────────────────────────────────────────────────────
// Woob JSON output types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WoobAccount {
    id: String,
    label: Option<String>,
    balance: Option<f64>,
    currency: Option<String>,
    #[serde(rename = "type")]
    account_type: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct WoobTransaction {
    id: String,
    date: Option<String>,
    label: Option<String>,
    amount: Option<serde_json::Value>, // can be string or number depending on woob version
    raw: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public result types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankAccount {
    pub id: String,
    pub label: String,
    pub balance: f64,
    pub currency: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankSyncResult {
    pub wf_account_id: String,
    pub transactions_imported: usize,
    pub transactions_skipped: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: find the woob binary
// ─────────────────────────────────────────────────────────────────────────────

fn find_woob() -> Result<String, String> {
    // Check common macOS Python user install locations first
    let home = std::env::var("HOME").unwrap_or_default();
    let python_versions = ["3.14", "3.13", "3.12", "3.11", "3.10"];
    for version in &python_versions {
        let path = format!("{}/Library/Python/{}/bin/woob", home, version);
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // Try common system locations
    for candidate in &[
        "/usr/local/bin/woob",
        "/usr/bin/woob",
        "/opt/homebrew/bin/woob",
    ] {
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }

    Err("Woob not found. Install with: pip3 install woob".to_string())
}

/// Run a woob command and return stdout as String.
async fn run_woob(args: &[&str]) -> Result<String, String> {
    let woob = find_woob()?;
    debug!("Running: {} {}", woob, args.join(" "));

    let output = Command::new(&woob)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to run woob: {}", e))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        return Err(format!("Woob error: {}", stderr));
    }

    Ok(stdout)
}

/// Parse an amount value that may come as a JSON string or number.
fn parse_amount(val: &serde_json::Value) -> Option<f64> {
    match val {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.replace(',', ".").parse().ok(),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Commands
// ─────────────────────────────────────────────────────────────────────────────

/// Check whether Woob is installed and return its path.
#[tauri::command]
pub async fn bank_sync_check_woob() -> Result<String, String> {
    find_woob()
}

/// List all bank accounts configured in Woob.
/// Optionally filter by backend (e.g. "boursorama").
#[tauri::command]
pub async fn bank_sync_list_accounts(backend: Option<String>) -> Result<Vec<BankAccount>, String> {
    let mut args = vec!["bank"];
    if let Some(ref b) = backend {
        args.push("-b");
        args.push(b.as_str());
    }
    args.extend_from_slice(&["-f", "json", "list"]);

    let json = run_woob(&args).await?;

    // Woob may prefix stderr warnings before the JSON — find the first '['
    let json_start = json.find('[').ok_or("No JSON output from woob list")?;
    let json_clean = &json[json_start..];

    let accounts: Vec<WoobAccount> = serde_json::from_str(json_clean)
        .map_err(|e| format!("Failed to parse woob accounts: {}", e))?;

    let result = accounts
        .into_iter()
        .map(|a| BankAccount {
            id: a.id.clone(),
            label: a.label.unwrap_or_else(|| a.id.clone()),
            balance: a.balance.unwrap_or(0.0),
            currency: a.currency.unwrap_or_else(|| "EUR".to_string()),
        })
        .collect();

    Ok(result)
}

/// Sync transactions from a Woob account into a Wealthfolio account.
///
/// - `woob_account_id`: the ID from `bank_sync_list_accounts` (e.g. "00040585568@boursorama")
/// - `wf_account_id`: Wealthfolio account UUID to import into
/// - `count`: max number of transactions to fetch (default 200)
#[tauri::command]
pub async fn bank_sync_sync_account(
    woob_account_id: String,
    wf_account_id: String,
    count: Option<u32>,
    state: State<'_, Arc<ServiceContext>>,
) -> Result<BankSyncResult, String> {
    let n = count.unwrap_or(200).to_string();
    info!(
        "Starting Woob sync: woob={} wf={}",
        woob_account_id, wf_account_id
    );

    let json = run_woob(&["bank", "-f", "json", "-n", &n, "history", &woob_account_id]).await?;

    // Find the JSON array start (woob may print warnings before it)
    let json_start = json.find('[').ok_or("No JSON output from woob history")?;
    let json_clean = &json[json_start..];

    let transactions: Vec<WoobTransaction> = serde_json::from_str(json_clean)
        .map_err(|e| format!("Failed to parse woob transactions: {}", e))?;

    let total = transactions.len();
    info!("Fetched {} transactions from Woob", total);

    if transactions.is_empty() {
        return Ok(BankSyncResult {
            wf_account_id,
            transactions_imported: 0,
            transactions_skipped: 0,
        });
    }

    // Get account currency from Wealthfolio
    let wf_account = state
        .account_service()
        .get_account(&wf_account_id)
        .map_err(|e| format!("Failed to get Wealthfolio account: {}", e))?;

    let currency = wf_account.currency.clone();

    let activities: Vec<wealthfolio_connect::AccountUniversalActivity> = transactions
        .into_iter()
        .filter_map(|tx| map_woob_transaction(tx, &currency))
        .collect();

    let mapped = activities.len();
    let skipped = total - mapped;

    if activities.is_empty() {
        return Ok(BankSyncResult {
            wf_account_id,
            transactions_imported: 0,
            transactions_skipped: skipped,
        });
    }

    let (imported, _assets_created, _new_asset_ids, _needs_review) = state
        .sync_service()
        .upsert_account_activities(wf_account_id.clone(), None, activities)
        .await
        .map_err(|e| format!("Failed to import transactions: {}", e))?;

    info!(
        "Woob sync complete: {} imported, {} skipped",
        imported, skipped
    );

    Ok(BankSyncResult {
        wf_account_id,
        transactions_imported: imported,
        transactions_skipped: skipped,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Transaction mapping
// ─────────────────────────────────────────────────────────────────────────────

fn map_woob_transaction(
    tx: WoobTransaction,
    currency: &str,
) -> Option<wealthfolio_connect::AccountUniversalActivity> {
    let amount = tx.amount.as_ref().and_then(parse_amount)?;

    if amount == 0.0 {
        return None;
    }

    let date = tx.date?;

    let (activity_type, abs_amount) = if amount > 0.0 {
        ("DEPOSIT", amount)
    } else {
        ("WITHDRAWAL", amount.abs())
    };

    let description = tx.label.or(tx.raw).unwrap_or_default();

    Some(wealthfolio_connect::AccountUniversalActivity {
        id: Some(tx.id.clone()),
        source_record_id: Some(tx.id),
        source_system: Some("WOOB".to_string()),
        activity_type: Some(activity_type.to_string()),
        trade_date: Some(date),
        settlement_date: None,
        price: Some(abs_amount),
        units: Some(1.0),
        amount: Some(abs_amount),
        currency: Some(AccountUniversalActivityCurrency {
            id: None,
            code: Some(currency.to_string()),
            name: None,
        }),
        description: Some(description),
        fee: Some(0.0),
        symbol: None,
        option_symbol: None,
        subtype: None,
        raw_type: None,
        option_type: None,
        fx_rate: None,
        institution: None,
        external_reference_id: None,
        provider_type: None,
        source_group_id: None,
        mapping_metadata: None,
        needs_review: false,
    })
}
