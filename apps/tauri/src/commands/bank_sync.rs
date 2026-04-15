//! Tauri commands for local bank sync via Woob (https://woob.tech).
//!
//! Woob is an open-source Python library that scrapes French bank websites
//! directly — no API key, no subscription, no cloud account required.
//!
//! Prerequisites (user side):
//!   pip3 install woob
//!   woob bank backends add boursorama   (or any other supported bank)
//!
//! Credentials are stored in the OS keychain (macOS Keychain, Windows
//! Credential Manager, Linux Secret Service) via Woob's "c" (command) option.
//! The `bank_sync_migrate_to_keychain` command performs a one-time migration
//! from plain-text "s" storage to keychain-backed "c" storage.

use keyring::Entry;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::process::Command;
use uuid::Uuid;
use wealthfolio_connect::broker::{
    AccountUniversalActivityCurrency, AccountUniversalActivitySymbol,
};
use wealthfolio_core::activities::{ActivityBulkMutationRequest, NewActivity};

use crate::context::ServiceContext;

// ─────────────────────────────────────────────────────────────────────────────
// Woob JSON output types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WoobAccount {
    id: String,
    label: Option<String>,
    balance: Option<serde_json::Value>,
    currency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WoobInvestment {
    label: Option<String>,
    code: Option<String>,
    stock_symbol: Option<String>,
    quantity: Option<serde_json::Value>,
    unitvalue: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WoobTransaction {
    id: String,
    date: Option<String>,
    label: Option<String>,
    amount: Option<serde_json::Value>,
    raw: Option<String>,
    #[serde(rename = "type")]
    tx_type: Option<i64>,
    commission: Option<serde_json::Value>,
    investments: Option<Vec<WoobInvestment>>,
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
    /// Date (YYYY-MM-DD) of the most recent transaction in this batch.
    /// Used by the frontend to know where to start the next sync.
    pub latest_transaction_date: Option<String>,
}

/// A candidate transfer pair found by `bank_sync_find_transfer_pairs`.
/// Nothing is modified until the user confirms via `bank_sync_apply_transfer_pairs`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPairCandidate {
    /// Ephemeral ID for this candidate (used by the frontend to track selection).
    pub id: String,
    pub out_activity_id: String,
    pub out_account_id: String,
    pub out_account_name: String,
    pub out_date: String,
    pub in_activity_id: String,
    pub in_account_id: String,
    pub in_account_name: String,
    pub in_date: String,
    pub amount: f64,
    pub currency: String,
    /// true = exact same date; false = ±1 day (needs user scrutiny)
    pub same_day: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyTransferPairsResult {
    pub pairs_linked: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPairSelection {
    pub out_activity_id: String,
    pub in_activity_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BankModule {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleField {
    pub name: String,
    pub label: String,
    pub masked: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredBackend {
    pub name: String,
    pub module: String,
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

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        return Err(format!("Woob error: {}", stderr.trim()));
    }

    Ok(stdout)
}

/// Find the python3 interpreter that has Woob installed.
///
/// Reads the shebang line from the Woob binary so we always use the same
/// Python that Woob was installed into, regardless of which python3 is on PATH.
fn find_python() -> String {
    // Primary: read the shebang from the Woob binary
    if let Ok(woob_path) = find_woob() {
        if let Ok(content) = std::fs::read_to_string(&woob_path) {
            if let Some(first_line) = content.lines().next() {
                if let Some(shebang) = first_line.strip_prefix("#!") {
                    let parts: Vec<&str> = shebang.split_whitespace().collect();
                    let python_name = if parts.len() >= 2 && parts[0].ends_with("env") {
                        // e.g. "#!/usr/bin/env python3"
                        parts[1]
                    } else {
                        // e.g. "#!/usr/local/bin/python3"
                        parts[0]
                    };
                    if std::path::Path::new(python_name).exists() {
                        return python_name.to_string();
                    }
                    // Relative name — search common dirs
                    for dir in &["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"] {
                        let p = format!("{}/{}", dir, python_name);
                        if std::path::Path::new(&p).exists() {
                            return p;
                        }
                    }
                }
            }
        }
    }

    // Fallback: try likely locations
    for candidate in &[
        "/usr/local/bin/python3",
        "/opt/homebrew/bin/python3",
        "/usr/bin/python3",
    ] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }

    "python3".to_string()
}

/// Run a Python snippet and return stdout.
async fn run_python(script: &str) -> Result<String, String> {
    let python = find_python();
    let output = Command::new(&python)
        .args(["-c", script])
        .output()
        .await
        .map_err(|e| format!("Failed to run python3: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Python error: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
            balance: a.balance.as_ref().and_then(parse_amount).unwrap_or(0.0),
            currency: a.currency.unwrap_or_else(|| "EUR".to_string()),
        })
        .collect();

    Ok(result)
}

/// Fetch Woob history for a date range using an iterative strategy.
///
/// Woob modules return ~10 transactions per call by default (one page).
/// We start with a small batch and triple it on each retry until we either:
///   - reach a transaction older than `since_date` (we have everything we need), or
///   - get fewer results than requested (bank has no more history), or
///   - hit the safety cap of 5000.
///
/// A 1-second pause between retries avoids triggering bank rate-limiting.
async fn fetch_woob_history(
    woob_account_id: &str,
    since_date: Option<&str>,
    until_date: Option<&str>,
) -> Result<Vec<WoobTransaction>, String> {
    const INITIAL_BATCH: u64 = 200;
    const MAX_N: u64 = 5000;

    let mut n = INITIAL_BATCH;

    loop {
        let n_str = n.to_string();
        info!("Fetching Woob history: n={} account={}", n, woob_account_id);

        let mut args = vec!["bank", "-f", "json", "-n", &n_str];
        if let Some(since) = since_date {
            args.extend_from_slice(&["--since", since]);
        }
        args.extend_from_slice(&["history", woob_account_id]);

        let json = run_woob(&args).await?;
        let json_start = json.find('[').ok_or("No JSON output from woob history")?;
        let mut transactions: Vec<WoobTransaction> = serde_json::from_str(&json[json_start..])
            .map_err(|e| format!("Failed to parse woob transactions: {}", e))?;

        let fetched = transactions.len() as u64;

        // Apply until_date filter so oldest-date check below is within range
        if let Some(until) = until_date {
            transactions.retain(|tx| tx.date.as_deref().map(|d| d <= until).unwrap_or(true));
        }

        // Find the oldest date in this batch
        let oldest = transactions
            .iter()
            .filter_map(|tx| tx.date.as_deref())
            .min()
            .map(str::to_string);

        let reached_since = match (&oldest, since_date) {
            (Some(oldest_date), Some(since)) => oldest_date.as_str() <= since,
            _ => true,
        };
        let bank_exhausted = fetched < n;

        if reached_since || bank_exhausted || n >= MAX_N {
            // Apply since_date filter and return
            if let Some(since) = since_date {
                transactions.retain(|tx| tx.date.as_deref().map(|d| d >= since).unwrap_or(true));
            }
            info!(
                "History fetch complete: {} transactions in range (oldest={:?}, retried_with_n={})",
                transactions.len(),
                oldest,
                n
            );
            return Ok(transactions);
        }

        // Need to go further back — triple the batch and wait to avoid rate-limiting
        n = (n * 3).min(MAX_N);
        info!("Need more history, retrying with n={} (pausing 1s)", n);
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Sync transactions from a Woob account into a Wealthfolio account.
///
/// - `woob_account_id`: the ID from `bank_sync_list_accounts` (e.g. "00040585568@boursorama")
/// - `wf_account_id`: Wealthfolio account UUID to import into
#[tauri::command]
pub async fn bank_sync_sync_account(
    woob_account_id: String,
    wf_account_id: String,
    since_date: Option<String>,
    until_date: Option<String>,
    state: State<'_, Arc<ServiceContext>>,
) -> Result<BankSyncResult, String> {
    info!(
        "Starting Woob sync: woob={} wf={} since={:?} until={:?}",
        woob_account_id, wf_account_id, since_date, until_date
    );

    let transactions = fetch_woob_history(
        &woob_account_id,
        since_date.as_deref(),
        until_date.as_deref(),
    )
    .await?;

    let total = transactions.len();
    info!("Fetched {} transactions from Woob", total);

    // Capture the most recent transaction date before consuming the vec.
    let latest_transaction_date = transactions
        .iter()
        .filter_map(|tx| tx.date.as_deref())
        .max()
        .map(str::to_string);

    if transactions.is_empty() {
        return Ok(BankSyncResult {
            wf_account_id,
            transactions_imported: 0,
            transactions_skipped: 0,
            latest_transaction_date: None,
        });
    }

    // Get account currency from Wealthfolio
    let wf_account = state
        .account_service()
        .get_account(&wf_account_id)
        .map_err(|e| format!("Failed to get Wealthfolio account: {}", e))?;

    let currency = wf_account.currency.clone();

    // Build a fingerprint set of non-Woob activities (manual entries, CSV imports…)
    // to detect duplicates by (type, date, amount) — covers the case where the user
    // manually entered a transaction that Woob would also sync.
    let existing_fingerprints: std::collections::HashSet<(String, String, String)> = state
        .activity_service()
        .get_activities_by_account_id(&wf_account_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|act| {
            // Skip activities already from Woob — those are handled by the upsert ID match
            act.source_system.as_deref() != Some("WOOB")
        })
        .filter_map(|act| {
            let amount = act.amount?;
            // Date: take first 10 chars of DateTime string → "YYYY-MM-DD"
            let date = act.activity_date.to_string();
            let date_str = date.get(..10)?.to_string();
            Some((act.activity_type, date_str, format!("{:.2}", amount)))
        })
        .collect();

    let activities: Vec<wealthfolio_connect::AccountUniversalActivity> = transactions
        .into_iter()
        .filter_map(|tx| map_woob_transaction(tx, &currency))
        .filter(|act| {
            // Fuzzy dedup: skip if (type, date, amount) already exists as a non-Woob activity
            let Some(ref trade_date) = act.trade_date else {
                return true;
            };
            let date_str = trade_date.get(..10).unwrap_or(trade_date.as_str());
            let activity_type = act.activity_type.as_deref().unwrap_or("");
            let amount_str = act.amount.map(|a| format!("{:.2}", a)).unwrap_or_default();
            let fingerprint = (activity_type.to_string(), date_str.to_string(), amount_str);
            if existing_fingerprints.contains(&fingerprint) {
                debug!(
                    "Skipping duplicate: {} {} {}",
                    fingerprint.0, fingerprint.1, fingerprint.2
                );
                return false;
            }
            true
        })
        .collect();

    // Remove cash debits/credits that are the counterpart of an investment order.
    // Fortuneo generates both a cash movement AND a market transaction for each
    // "Achat/Vente Comptant", so we deduplicate by (date, amount).
    let investment_keys: std::collections::HashSet<(String, String)> = activities
        .iter()
        .filter(|a| matches!(a.activity_type.as_deref(), Some("BUY") | Some("SELL")))
        .filter_map(|a| {
            let date = a.trade_date.as_ref()?.get(..10)?.to_string();
            let amount = format!("{:.2}", a.amount?);
            Some((date, amount))
        })
        .collect();

    let activities: Vec<_> = activities
        .into_iter()
        .filter(|a| {
            if matches!(
                a.activity_type.as_deref(),
                Some("DEPOSIT") | Some("WITHDRAWAL")
            ) {
                if let (Some(date), Some(amount)) = (&a.trade_date, a.amount) {
                    let key = (
                        date.get(..10).unwrap_or(date.as_str()).to_string(),
                        format!("{:.2}", amount),
                    );
                    return !investment_keys.contains(&key);
                }
            }
            true
        })
        .collect();

    let mapped = activities.len();
    let skipped = total - mapped;

    if activities.is_empty() {
        return Ok(BankSyncResult {
            wf_account_id,
            transactions_imported: 0,
            transactions_skipped: skipped,
            latest_transaction_date,
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
        latest_transaction_date,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Backend management commands
// ─────────────────────────────────────────────────────────────────────────────

/// List all available bank modules from Woob repositories (~96 French banks).
#[tauri::command]
pub async fn bank_sync_list_modules() -> Result<Vec<BankModule>, String> {
    let script = r#"
import json, sys
try:
    from woob.core import Woob
    w = Woob()
    modules = w.repositories.get_all_modules_info()
    result = [
        {'name': name, 'description': info.description or name}
        for name, info in sorted(modules.items())
        if 'CapBank' in str(info.capabilities)
    ]
    print(json.dumps(result))
except Exception as e:
    print(str(e), file=sys.stderr)
    sys.exit(1)
"#;

    let output = run_python(script).await?;
    serde_json::from_str::<Vec<BankModule>>(output.trim())
        .map_err(|e| format!("Failed to parse modules list: {}", e))
}

/// Get the persistent configuration fields for a Woob module.
/// Excludes transient fields (OTP codes, session tokens) — those are handled interactively by Woob.
#[tauri::command]
pub async fn bank_sync_module_config(module: String) -> Result<Vec<ModuleField>, String> {
    let module_repr = serde_json::to_string(&module).unwrap();
    let script = format!(
        r#"
import json, sys
try:
    from woob.core import Woob
    w = Woob()
    m = w.load_or_install_module({module_repr})
    result = [
        {{
            'name': fname,
            'label': getattr(field, 'description', fname) or fname,
            'masked': bool(getattr(field, 'masked', False)),
        }}
        for fname, field in m.klass.CONFIG.items()
        if field.__class__.__name__ != 'ValueTransient'
    ]
    print(json.dumps(result))
except Exception as e:
    print(str(e), file=sys.stderr)
    sys.exit(1)
"#,
        module_repr = module_repr
    );

    let output = run_python(&script).await?;
    serde_json::from_str::<Vec<ModuleField>>(output.trim())
        .map_err(|e| format!("Failed to parse module config: {}", e))
}

/// Return all backends currently configured in ~/.config/woob/backends.
#[tauri::command]
pub async fn bank_sync_list_configured_backends() -> Result<Vec<ConfiguredBackend>, String> {
    let path = woob_backends_path()?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read backends file: {}", e))?;

    let result = parse_ini(&content)
        .into_iter()
        .map(|(name, fields)| {
            let module = fields
                .iter()
                .find(|(k, _)| k == "_module")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| name.clone());
            ConfiguredBackend { name, module }
        })
        .collect();

    Ok(result)
}

/// Configure a new Woob backend, storing credentials directly in the OS keychain.
/// Credentials never touch disk as plain text.
///
/// If a backend with the same name already exists it is replaced.
#[tauri::command]
pub async fn bank_sync_setup_backend(
    backend_name: String,
    module: String,
    credentials: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let helper_path = write_helper_script()?;
    let path = woob_backends_path()?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create woob config dir: {}", e))?;
    }

    // Use absolute Python path — Woob runs "c" commands via /bin/sh which has a
    // limited PATH that won't include /usr/local/bin or /opt/homebrew/bin.
    let python = find_python();

    // Load existing backends, dropping any with the same name (re-setup)
    let existing = if path.exists() {
        std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read backends file: {}", e))?
    } else {
        String::new()
    };
    let mut sections = parse_ini(&existing);
    sections.retain(|(name, _)| name != &backend_name);

    // Build the new section: _module first, then each credential field
    let mut new_fields: Vec<(String, String)> = vec![("_module".to_string(), module.clone())];

    for (field, value) in &credentials {
        let keychain_key = format!(
            "woob_{}_{}",
            backend_name.to_lowercase(),
            field.to_lowercase()
        );
        store_in_keychain(&keychain_key, value)?;
        info!("Stored credential '{}' in keychain", keychain_key);

        // Woob executes backtick-wrapped values as shell commands (backendscfg.py:47)
        let cmd = format!("`{} {} {}`", python, helper_path, keychain_key);
        new_fields.push((field.clone(), cmd));
    }

    sections.push((backend_name.clone(), new_fields));

    // Serialize all sections back to INI
    let mut content = String::new();
    for (section, fields) in &sections {
        content.push_str(&format!("[{}]\n", section));
        for (k, v) in fields {
            content.push_str(&format!("{} = {}\n", k, v));
        }
        content.push('\n');
    }

    std::fs::write(&path, &content).map_err(|e| format!("Failed to write backends file: {}", e))?;
    info!(
        "Backend '{}' (module: {}) set up successfully",
        backend_name, module
    );

    Ok(())
}

/// Delete a configured backend: removes it from the backends file and wipes its keychain entries.
#[tauri::command]
pub async fn bank_sync_delete_backend(backend_name: String) -> Result<(), String> {
    let path = woob_backends_path()?;
    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read backends file: {}", e))?;
    let mut sections = parse_ini(&content);

    let mut credential_keys: Vec<String> = Vec::new();
    sections.retain(|(name, fields)| {
        if name != &backend_name {
            return true;
        }
        for (key, _) in fields {
            if !key.starts_with('_') {
                credential_keys.push(format!(
                    "woob_{}_{}",
                    backend_name.to_lowercase(),
                    key.to_lowercase()
                ));
            }
        }
        false
    });

    // Delete keychain entries (best effort — don't fail if already gone)
    for keychain_key in &credential_keys {
        let service = format!("wealthfolio_{}", keychain_key.to_lowercase());
        match Entry::new(&service, "default") {
            Ok(entry) => {
                let _ = entry.delete_password();
                info!("Deleted keychain entry '{}'", service);
            }
            Err(e) => debug!("Could not delete keychain entry '{}': {}", service, e),
        }
    }

    // Write updated file
    let mut new_content = String::new();
    for (section, fields) in &sections {
        new_content.push_str(&format!("[{}]\n", section));
        for (k, v) in fields {
            new_content.push_str(&format!("{} = {}\n", k, v));
        }
        new_content.push('\n');
    }

    std::fs::write(&path, &new_content)
        .map_err(|e| format!("Failed to write backends file: {}", e))?;
    info!("Backend '{}' deleted", backend_name);

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Keychain migration
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the path to ~/.config/woob/backends.
fn woob_backends_path() -> Result<std::path::PathBuf, String> {
    let home =
        std::env::var("HOME").map_err(|_| "HOME environment variable not set".to_string())?;
    Ok(std::path::PathBuf::from(home).join(".config/woob/backends"))
}

/// Parse a Woob INI backends file into sections.
/// Returns Vec<(section_name, Vec<(key, value)>)> preserving order.
fn parse_ini(content: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut current: Option<(String, Vec<(String, String)>)> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(s) = current.take() {
                sections.push(s);
            }
            current = Some((line[1..line.len() - 1].to_string(), Vec::new()));
        } else if let Some((_, ref mut fields)) = current {
            if let Some(pos) = line.find('=') {
                let key = line[..pos].trim().to_string();
                let value = line[pos + 1..].trim().to_string();
                if !key.is_empty() {
                    fields.push((key, value));
                }
            }
        }
    }
    if let Some(s) = current {
        sections.push(s);
    }
    sections
}

/// Check whether a parsed INI section has any plain-text credentials.
/// A field is considered migrated if its value is a backtick-wrapped command.
fn section_has_plain_text(fields: &[(String, String)]) -> bool {
    for (key, value) in fields {
        if key.starts_with('_') {
            continue;
        }
        // Backtick-wrapped values are commands — already using keychain
        if value.starts_with('`') && value.ends_with('`') {
            continue;
        }
        return true;
    }
    false
}

/// Write the cross-platform Python helper script that reads secrets from the
/// OS keychain and returns them to Woob via stdout.
fn write_helper_script() -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let config_dir = std::path::PathBuf::from(&home).join(".config/woob");
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create woob config dir: {}", e))?;

    let script_path = config_dir.join("wealthfolio_helper.py");

    let script = r#"#!/usr/bin/env python3
"""Wealthfolio Woob credential helper.

Reads secrets from the OS keychain and prints them to stdout.
Woob calls this script via the "c" backend option.

Usage: wealthfolio_helper.py <key>
  key: e.g. woob_boursorama_login
"""
import sys
import subprocess
import platform


def main():
    if len(sys.argv) < 2:
        print("Usage: wealthfolio_helper.py <key>", file=sys.stderr)
        sys.exit(1)

    key = sys.argv[1]
    # Must match format_service_id() in the Rust secret_store:
    # wealthfolio_{key.to_lowercase()}
    service = f"wealthfolio_{key.lower()}"
    account = "default"
    system = platform.system()

    if system == "Darwin":
        r = subprocess.run(
            ["security", "find-generic-password", "-s", service, "-a", account, "-w"],
            capture_output=True,
            text=True,
        )
    elif system == "Windows":
        ps = (
            f"$vault = New-Object Windows.Security.Credentials.PasswordVault;"
            f"$c = $vault.Retrieve('{service}', '{account}');"
            f"$c.RetrievePassword(); Write-Output $c.Password"
        )
        r = subprocess.run(
            ["powershell", "-NoProfile", "-Command", ps],
            capture_output=True,
            text=True,
        )
    else:
        # Linux — requires libsecret / secret-tool
        r = subprocess.run(
            ["secret-tool", "lookup", "service", service, "account", account],
            capture_output=True,
            text=True,
        )

    if r.returncode != 0:
        print(
            f"wealthfolio_helper: credential '{key}' not found in keychain.",
            file=sys.stderr,
        )
        sys.exit(1)

    # Print without trailing newline — Woob reads exactly what is printed
    print(r.stdout.strip(), end="")


if __name__ == "__main__":
    main()
"#;

    std::fs::write(&script_path, script)
        .map_err(|e| format!("Failed to write helper script: {}", e))?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to chmod helper script: {}", e))?;
    }

    Ok(script_path.to_string_lossy().to_string())
}

/// Store a credential in the OS keychain.
///
/// On macOS we use the `security` CLI tool instead of the `keyring` crate so that
/// the created item's ACL allows `security` to read it later (from the Woob helper
/// script) without triggering macOS authorization dialogs. Using the `keyring` crate
/// creates items owned by the Tauri process, which blocks cross-app access.
fn store_in_keychain(key: &str, value: &str) -> Result<(), String> {
    let service = format!("wealthfolio_{}", key.to_lowercase());

    #[cfg(target_os = "macos")]
    {
        // Delete first so re-setup doesn't fail with "duplicate item"
        let _ = std::process::Command::new("security")
            .args(["delete-generic-password", "-s", &service, "-a", "default"])
            .output();

        let out = std::process::Command::new("security")
            .args([
                "add-generic-password",
                "-s",
                &service,
                "-a",
                "default",
                "-w",
                value,
            ])
            .output()
            .map_err(|e| format!("Failed to run security: {}", e))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("Keychain error for '{}': {}", key, stderr.trim()));
        }
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        let entry = Entry::new(&service, "default")
            .map_err(|e| format!("Keyring init error for '{}': {}", key, e))?;
        entry
            .set_password(value)
            .map_err(|e| format!("Failed to store '{}' in keychain: {}", key, e))
    }
}

/// Check whether migration is needed (any backend still uses plain-text "s" storage).
#[tauri::command]
pub async fn bank_sync_needs_migration() -> Result<bool, String> {
    let path = woob_backends_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read backends file: {}", e))?;
    let sections = parse_ini(&content);
    Ok(sections
        .iter()
        .any(|(_, fields)| section_has_plain_text(fields)))
}

/// Migrate all plain-text Woob credentials to the OS keychain.
///
/// For each backend field stored as plain text ("s"):
///   1. Stores the value in the OS keychain under "woob_{backend}_{field}".
///   2. Replaces the value with a command that calls the Python helper script.
///   3. Sets `_{field} = c` in the backends file.
///
/// Creates a `.bak` backup of the backends file before modifying it.
#[tauri::command]
pub async fn bank_sync_migrate_to_keychain() -> Result<(), String> {
    let path = woob_backends_path()?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read backends file: {}", e))?;

    let helper_path = write_helper_script()?;
    let python = find_python();
    let sections = parse_ini(&content);

    let mut new_content = String::new();

    for (section_name, fields) in &sections {
        new_content.push_str(&format!("[{}]\n", section_name));

        for (key, value) in fields {
            if key.starts_with('_') {
                // Only keep _module — drop legacy _field = c/p/s markers
                if key == "_module" {
                    new_content.push_str(&format!("{} = {}\n", key, value));
                }
                continue;
            }

            // Already migrated (backtick command) — keep as-is
            if value.starts_with('`') && value.ends_with('`') {
                new_content.push_str(&format!("{} = {}\n", key, value));
                continue;
            }

            // Plain-text — migrate to keychain
            let keychain_key = format!(
                "woob_{}_{}",
                section_name.to_lowercase(),
                key.to_lowercase()
            );
            store_in_keychain(&keychain_key, value)?;
            info!("Migrated '{}' to keychain as '{}'", key, keychain_key);

            let cmd = format!("`{} {} {}`", python, helper_path, keychain_key);
            new_content.push_str(&format!("{} = {}\n", key, cmd));
        }

        new_content.push('\n');
    }

    // Back up then overwrite
    let backup = path.with_extension("bak");
    std::fs::copy(&path, &backup)
        .map_err(|e| format!("Failed to create backup ({:?}): {}", backup, e))?;
    info!("Backed up backends file to {:?}", backup);

    std::fs::write(&path, &new_content)
        .map_err(|e| format!("Failed to write updated backends file: {}", e))?;
    info!("Credentials migrated to keychain successfully");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Transfer pair detection & linking
// ─────────────────────────────────────────────────────────────────────────────

/// Scan all activities and return candidate DEPOSIT/WITHDRAWAL pairs that look
/// like internal transfers (same |amount| + currency across two different accounts,
/// same day or ±1 day). Nothing is modified.
#[tauri::command]
pub async fn bank_sync_find_transfer_pairs(
    state: State<'_, Arc<ServiceContext>>,
) -> Result<Vec<TransferPairCandidate>, String> {
    let all_activities = state
        .activity_service()
        .get_activities()
        .map_err(|e| format!("Failed to load activities: {}", e))?;

    let accounts: std::collections::HashMap<String, String> = state
        .account_service()
        .get_all_accounts()
        .map_err(|e| format!("Failed to load accounts: {}", e))?
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect();

    // Only consider unlinked cash flows (no source_group_id already set)
    let candidates: Vec<_> = all_activities
        .into_iter()
        .filter(|a| {
            a.source_group_id.is_none()
                && matches!(
                    a.activity_type.as_str(),
                    "DEPOSIT" | "WITHDRAWAL" | "TRANSFER_IN" | "TRANSFER_OUT"
                )
        })
        .collect();

    // Outflows: WITHDRAWAL or TRANSFER_OUT (money leaving an account)
    // Inflows:  DEPOSIT or TRANSFER_IN   (money entering an account)
    let outflows: Vec<_> = candidates
        .iter()
        .filter(|a| matches!(a.activity_type.as_str(), "WITHDRAWAL" | "TRANSFER_OUT"))
        .collect();
    let inflows: Vec<_> = candidates
        .iter()
        .filter(|a| matches!(a.activity_type.as_str(), "DEPOSIT" | "TRANSFER_IN"))
        .collect();

    let mut pairs: Vec<TransferPairCandidate> = Vec::new();
    // Track which inflow IDs have already been matched (one inflow per outflow)
    let mut matched_in: std::collections::HashSet<String> = std::collections::HashSet::new();

    for out in &outflows {
        let out_amount = match out.amount {
            Some(a) if !a.is_zero() => a,
            _ => continue,
        };
        let out_date = out.activity_date.date_naive();

        for infl in &inflows {
            if matched_in.contains(&infl.id) {
                continue;
            }
            if infl.account_id == out.account_id {
                continue; // must be different accounts
            }
            if infl.currency != out.currency {
                continue;
            }
            let in_amount = match infl.amount {
                Some(a) if !a.is_zero() => a,
                _ => continue,
            };
            if in_amount != out_amount {
                continue;
            }

            let in_date = infl.activity_date.date_naive();
            let day_diff = (out_date - in_date).num_days().abs();
            if day_diff > 1 {
                continue;
            }

            matched_in.insert(infl.id.clone());
            pairs.push(TransferPairCandidate {
                id: Uuid::new_v4().to_string(),
                out_activity_id: out.id.clone(),
                out_account_id: out.account_id.clone(),
                out_account_name: accounts
                    .get(&out.account_id)
                    .cloned()
                    .unwrap_or_else(|| out.account_id.clone()),
                out_date: out_date.to_string(),
                in_activity_id: infl.id.clone(),
                in_account_id: infl.account_id.clone(),
                in_account_name: accounts
                    .get(&infl.account_id)
                    .cloned()
                    .unwrap_or_else(|| infl.account_id.clone()),
                in_date: in_date.to_string(),
                amount: out_amount.to_string().parse::<f64>().unwrap_or_default(),
                currency: out.currency.clone(),
                same_day: day_diff == 0,
            });
            break; // one match per outflow
        }
    }

    Ok(pairs)
}

/// Apply the user-confirmed transfer pairs: reclassify DEPOSIT→TRANSFER_IN and
/// WITHDRAWAL→TRANSFER_OUT, link them with a shared source_group_id.
///
/// Strategy: delete the old activities and recreate them with the correct type
/// and source_group_id (ActivityUpdate does not expose source_group_id).
#[tauri::command]
pub async fn bank_sync_apply_transfer_pairs(
    pairs: Vec<TransferPairSelection>,
    state: State<'_, Arc<ServiceContext>>,
) -> Result<ApplyTransferPairsResult, String> {
    if pairs.is_empty() {
        return Ok(ApplyTransferPairsResult { pairs_linked: 0 });
    }

    // Load all needed activities in one call
    let all_ids: Vec<String> = pairs
        .iter()
        .flat_map(|p| [p.out_activity_id.clone(), p.in_activity_id.clone()])
        .collect();

    let all_activities = state
        .activity_service()
        .get_activities()
        .map_err(|e| format!("Failed to load activities: {}", e))?;

    let by_id: std::collections::HashMap<String, wealthfolio_core::activities::Activity> =
        all_activities
            .into_iter()
            .filter(|a| all_ids.contains(&a.id))
            .map(|a| (a.id.clone(), a))
            .collect();

    // bulk_mutate_activities uses the first create's account_id for ALL creates in
    // the batch. Since OUT and IN belong to different accounts, we must split them
    // into per-account calls to avoid the early-return-on-error path.
    let mut count = 0;
    for pair in &pairs {
        let out_act = match by_id.get(&pair.out_activity_id) {
            Some(a) => a,
            None => return Err(format!("Activity not found: {}", pair.out_activity_id)),
        };
        let in_act = match by_id.get(&pair.in_activity_id) {
            Some(a) => a,
            None => return Err(format!("Activity not found: {}", pair.in_activity_id)),
        };

        let group_id = Uuid::new_v4().to_string();

        // Call 1: OUT side (delete old WITHDRAWAL/TRANSFER_OUT, create TRANSFER_OUT)
        let result_out = state
            .activity_service()
            .bulk_mutate_activities(ActivityBulkMutationRequest {
                creates: vec![activity_as_transfer(out_act, "TRANSFER_OUT", &group_id)],
                updates: vec![],
                delete_ids: vec![out_act.id.clone()],
            })
            .await
            .map_err(|e| format!("Failed to apply OUT side: {}", e))?;

        if !result_out.errors.is_empty() {
            return Err(format!(
                "Failed to apply OUT side: {}",
                result_out.errors[0].message
            ));
        }

        // Call 2: IN side (delete old DEPOSIT/TRANSFER_IN, create TRANSFER_IN)
        let result_in = state
            .activity_service()
            .bulk_mutate_activities(ActivityBulkMutationRequest {
                creates: vec![activity_as_transfer(in_act, "TRANSFER_IN", &group_id)],
                updates: vec![],
                delete_ids: vec![in_act.id.clone()],
            })
            .await
            .map_err(|e| format!("Failed to apply IN side: {}", e))?;

        if !result_in.errors.is_empty() {
            return Err(format!(
                "Failed to apply IN side: {}",
                result_in.errors[0].message
            ));
        }

        count += 1;
    }

    Ok(ApplyTransferPairsResult {
        pairs_linked: count,
    })
}

/// Clone an Activity into a NewActivity, changing only the type and source_group_id.
fn activity_as_transfer(
    act: &wealthfolio_core::activities::Activity,
    new_type: &str,
    group_id: &str,
) -> NewActivity {
    NewActivity {
        id: None,
        account_id: act.account_id.clone(),
        symbol: None,
        activity_type: new_type.to_string(),
        subtype: None,
        activity_date: act.activity_date.to_rfc3339(),
        quantity: act.quantity,
        unit_price: act.unit_price,
        currency: act.currency.clone(),
        fee: act.fee,
        amount: act.amount,
        status: Some(act.status.clone()),
        notes: act.notes.clone(),
        fx_rate: act.fx_rate,
        metadata: act.metadata.as_ref().map(|m| m.to_string()),
        needs_review: Some(false),
        source_system: act.source_system.clone(),
        source_record_id: act.source_record_id.clone(),
        source_group_id: Some(group_id.to_string()),
        idempotency_key: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transaction mapping
// ─────────────────────────────────────────────────────────────────────────────

fn map_woob_transaction(
    tx: WoobTransaction,
    currency: &str,
) -> Option<wealthfolio_connect::AccountUniversalActivity> {
    let amount = tx.amount.as_ref().and_then(parse_amount)?;
    let date = tx.date?;
    let description = tx
        .label
        .clone()
        .or_else(|| tx.raw.clone())
        .unwrap_or_default();

    // Woob type 9 = MARKET (investment order: BUY/SELL)
    if tx.tx_type == Some(9) {
        let inv = tx.investments.as_ref().and_then(|v| v.first());

        let symbol = inv.and_then(|i| {
            i.stock_symbol
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| i.code.clone().filter(|s| !s.is_empty()))
                .or_else(|| i.label.clone().filter(|s| !s.is_empty()))
        });

        let quantity = inv
            .and_then(|i| i.quantity.as_ref())
            .and_then(parse_amount)
            .unwrap_or(1.0)
            .abs();

        let unit_price = inv
            .and_then(|i| i.unitvalue.as_ref())
            .and_then(parse_amount)
            .unwrap_or_else(|| {
                if quantity > 0.0 {
                    amount.abs() / quantity
                } else {
                    amount.abs()
                }
            });

        let fee = tx
            .commission
            .as_ref()
            .and_then(parse_amount)
            .unwrap_or(0.0)
            .abs();

        let activity_type = if amount < 0.0 { "BUY" } else { "SELL" };

        // Generate a stable unique ID since Woob reuses "@bank" as id for all investment transactions
        let stable_id = format!(
            "WOOB-{}-{}-{:.2}-{}",
            date,
            activity_type,
            amount.abs(),
            symbol.as_deref().unwrap_or(&description)
        );

        return Some(wealthfolio_connect::AccountUniversalActivity {
            id: Some(stable_id.clone()),
            source_record_id: Some(stable_id),
            source_system: Some("WOOB".to_string()),
            activity_type: Some(activity_type.to_string()),
            trade_date: Some(date),
            settlement_date: None,
            price: Some(unit_price),
            units: Some(quantity),
            amount: Some(amount.abs()),
            currency: Some(AccountUniversalActivityCurrency {
                id: None,
                code: Some(currency.to_string()),
                name: None,
            }),
            description: Some(description),
            fee: Some(fee),
            symbol: symbol.map(|s| AccountUniversalActivitySymbol {
                id: None,
                symbol: Some(s.clone()),
                raw_symbol: Some(s),
                description: None,
                symbol_type: None,
                exchange: None,
                currency: None,
                figi_code: None,
            }),
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
            needs_review: true, // needs review so user can assign the correct symbol
        });
    }

    // Regular cash transaction (DEPOSIT / WITHDRAWAL)
    if amount == 0.0 {
        return None;
    }

    let (activity_type, abs_amount) = if amount > 0.0 {
        ("DEPOSIT", amount)
    } else {
        ("WITHDRAWAL", amount.abs())
    };

    // Generate stable ID for cash transactions too (Woob reuses ids)
    let stable_id = if tx.id.contains('@') && !tx.id.starts_with("WOOB-") {
        format!("WOOB-{}-{}-{:.2}", date, activity_type, abs_amount)
    } else {
        tx.id.clone()
    };

    Some(wealthfolio_connect::AccountUniversalActivity {
        id: Some(stable_id.clone()),
        source_record_id: Some(stable_id),
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
