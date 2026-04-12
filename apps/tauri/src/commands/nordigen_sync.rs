//! Tauri commands for GoCardless (Nordigen) bank account sync.
//!
//! Provides free EU bank sync via the GoCardless Bank Account Data API (PSD2).
//! No Wealthfolio Connect subscription required — uses GoCardless directly.
//!
//! Auth flow:
//!   1. User creates a free GoCardless account at bankaccountdata.gocardless.com
//!   2. Addon calls nordigen_save_credentials with client_id + client_secret
//!   3. nordigen_create_requisition returns an OAuth link → open in browser
//!   4. User completes bank login
//!   5. nordigen_sync_account fetches + imports transactions

use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use wealthfolio_core::secrets::SecretStore;

use crate::context::ServiceContext;
use crate::secret_store::KeyringSecretStore;

const GOCARDLESS_BASE_URL: &str = "https://bankaccountdata.gocardless.com/api/v2";
const SECRET_CLIENT_ID: &str = "nordigen_client_id";
const SECRET_CLIENT_SECRET: &str = "nordigen_client_secret";
const SECRET_ACCESS_TOKEN: &str = "nordigen_access_token";
const SECRET_REFRESH_TOKEN: &str = "nordigen_refresh_token";

// ─────────────────────────────────────────────────────────────────────────────
// GoCardless API types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GcTokenResponse {
    access: String,
    refresh: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GcInstitution {
    pub id: String,
    pub name: String,
    pub bic: Option<String>,
    pub transaction_total_days: Option<String>,
    pub countries: Vec<String>,
    pub logo: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GcRequisition {
    pub id: String,
    pub status: String,
    pub link: String,
    pub accounts: Vec<String>,
    pub institution_id: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GcAccount {
    pub id: String,
    pub iban: Option<String>,
    pub currency: Option<String>,
    pub name: Option<String>,
    pub product: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GcAccountDetails {
    account: GcAccount,
}

#[derive(Debug, Deserialize)]
struct GcTransactionAmount {
    amount: String,
    currency: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GcTransaction {
    transaction_id: Option<String>,
    booking_date: Option<String>,
    value_date: Option<String>,
    transaction_amount: GcTransactionAmount,
    remittance_information_unstructured: Option<String>,
    remittance_information_structured: Option<String>,
    debtor_name: Option<String>,
    creditor_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GcTransactionList {
    booked: Vec<GcTransaction>,
}

#[derive(Debug, Deserialize)]
struct GcTransactionsResponse {
    transactions: GcTransactionList,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public result types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NordigenSyncResult {
    pub account_id: String,
    pub transactions_imported: usize,
    pub transactions_skipped: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to build HTTP client")
}

async fn get_access_token() -> Result<String, String> {
    // Try cached access token first
    if let Ok(Some(token)) = KeyringSecretStore.get_secret(SECRET_ACCESS_TOKEN) {
        if !token.is_empty() {
            return Ok(token);
        }
    }

    // No cached token — use refresh token to get a new one
    let refresh_token = KeyringSecretStore
        .get_secret(SECRET_REFRESH_TOKEN)
        .map_err(|e| e.to_string())?
        .ok_or("Not authenticated with GoCardless. Please connect first.")?;

    let client_id = KeyringSecretStore
        .get_secret(SECRET_CLIENT_ID)
        .map_err(|e| e.to_string())?
        .ok_or("GoCardless client ID not configured.")?;

    let client_secret = KeyringSecretStore
        .get_secret(SECRET_CLIENT_SECRET)
        .map_err(|e| e.to_string())?
        .ok_or("GoCardless client secret not configured.")?;

    let _ = client_id; // used to verify credentials are saved
    let _ = client_secret;

    let response = http_client()
        .post(format!("{}/token/refresh/", GOCARDLESS_BASE_URL))
        .json(&serde_json::json!({ "refresh": refresh_token }))
        .send()
        .await
        .map_err(|e| format!("Failed to refresh GoCardless token: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        // Token expired — clear so user re-authenticates
        let _ = KeyringSecretStore.delete_secret(SECRET_ACCESS_TOKEN);
        let _ = KeyringSecretStore.delete_secret(SECRET_REFRESH_TOKEN);
        return Err(format!(
            "GoCardless session expired ({}). Please reconnect.",
            body
        ));
    }

    let resp: GcTokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    KeyringSecretStore
        .set_secret(SECRET_ACCESS_TOKEN, &resp.access)
        .map_err(|e| e.to_string())?;

    Ok(resp.access)
}

// ─────────────────────────────────────────────────────────────────────────────
// Commands
// ─────────────────────────────────────────────────────────────────────────────

/// Save GoCardless API credentials and verify them by obtaining tokens.
/// Users get client_id + client_secret from bankaccountdata.gocardless.com → Developers.
#[tauri::command]
pub async fn nordigen_save_credentials(
    client_id: String,
    client_secret: String,
) -> Result<(), String> {
    KeyringSecretStore
        .set_secret(SECRET_CLIENT_ID, &client_id)
        .map_err(|e| e.to_string())?;
    KeyringSecretStore
        .set_secret(SECRET_CLIENT_SECRET, &client_secret)
        .map_err(|e| e.to_string())?;

    // Clear stale tokens
    let _ = KeyringSecretStore.delete_secret(SECRET_ACCESS_TOKEN);
    let _ = KeyringSecretStore.delete_secret(SECRET_REFRESH_TOKEN);

    // Verify credentials by obtaining fresh tokens
    let response = http_client()
        .post(format!("{}/token/new/", GOCARDLESS_BASE_URL))
        .json(&serde_json::json!({
            "secret_id": client_id,
            "secret_key": client_secret,
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to connect to GoCardless: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Invalid GoCardless credentials: {}", body));
    }

    let resp: GcTokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    KeyringSecretStore
        .set_secret(SECRET_ACCESS_TOKEN, &resp.access)
        .map_err(|e| e.to_string())?;

    if let Some(refresh) = resp.refresh {
        KeyringSecretStore
            .set_secret(SECRET_REFRESH_TOKEN, &refresh)
            .map_err(|e| e.to_string())?;
    }

    info!("GoCardless credentials saved and verified");
    Ok(())
}

/// Check whether GoCardless credentials are configured.
#[tauri::command]
pub async fn nordigen_check_credentials() -> Result<bool, String> {
    let has_id = KeyringSecretStore
        .get_secret(SECRET_CLIENT_ID)
        .map_err(|e| e.to_string())?
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    let has_refresh = KeyringSecretStore
        .get_secret(SECRET_REFRESH_TOKEN)
        .map_err(|e| e.to_string())?
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    Ok(has_id && has_refresh)
}

/// List available banks in a given country (ISO 3166-1 alpha-2, e.g. "FR").
#[tauri::command]
pub async fn nordigen_list_institutions(country: String) -> Result<Vec<GcInstitution>, String> {
    let token = get_access_token().await?;

    let response = http_client()
        .get(format!(
            "{}/institutions/?country={}",
            GOCARDLESS_BASE_URL, country
        ))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Failed to list institutions: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Failed to list institutions: {}", body));
    }

    response
        .json::<Vec<GcInstitution>>()
        .await
        .map_err(|e| format!("Failed to parse institutions: {}", e))
}

/// Create a bank connection (requisition) and return the OAuth link to open in browser.
#[tauri::command]
pub async fn nordigen_create_requisition(
    institution_id: String,
    redirect_uri: String,
) -> Result<GcRequisition, String> {
    let token = get_access_token().await?;

    let response = http_client()
        .post(format!("{}/requisitions/", GOCARDLESS_BASE_URL))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "redirect": redirect_uri,
            "institution_id": institution_id,
            "reference": format!("wealthfolio-{}", uuid::Uuid::new_v4()),
            "user_language": "FR",
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to create requisition: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Failed to create requisition: {}", body));
    }

    response
        .json::<GcRequisition>()
        .await
        .map_err(|e| format!("Failed to parse requisition: {}", e))
}

/// Get requisition status + linked account IDs after user completes bank OAuth.
#[tauri::command]
pub async fn nordigen_get_requisition(requisition_id: String) -> Result<GcRequisition, String> {
    let token = get_access_token().await?;

    let response = http_client()
        .get(format!(
            "{}/requisitions/{}/",
            GOCARDLESS_BASE_URL, requisition_id
        ))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Failed to get requisition: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Failed to get requisition: {}", body));
    }

    response
        .json::<GcRequisition>()
        .await
        .map_err(|e| format!("Failed to parse requisition: {}", e))
}

/// Get account details (IBAN, name, currency) for a GoCardless account ID.
#[tauri::command]
pub async fn nordigen_get_account_details(gc_account_id: String) -> Result<GcAccount, String> {
    let token = get_access_token().await?;

    let response = http_client()
        .get(format!(
            "{}/accounts/{}/details/",
            GOCARDLESS_BASE_URL, gc_account_id
        ))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Failed to get account details: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Failed to get account details: {}", body));
    }

    let details: GcAccountDetails = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse account details: {}", e))?;

    Ok(details.account)
}

/// Sync transactions from a GoCardless account into a Wealthfolio account.
/// - account_id: Wealthfolio account ID to import into
/// - gc_account_id: GoCardless account UUID (from nordigen_get_requisition)
/// - date_from: optional start date YYYY-MM-DD (defaults to 90 days ago)
#[tauri::command]
pub async fn nordigen_sync_account(
    account_id: String,
    gc_account_id: String,
    date_from: Option<String>,
    state: State<'_, Arc<ServiceContext>>,
) -> Result<NordigenSyncResult, String> {
    info!(
        "Starting Nordigen sync: wf_account={} gc_account={}",
        account_id, gc_account_id
    );

    let token = get_access_token().await?;

    let date_from = date_from.unwrap_or_else(|| {
        (chrono::Utc::now() - chrono::Duration::days(90))
            .format("%Y-%m-%d")
            .to_string()
    });

    let url = format!(
        "{}/accounts/{}/transactions/?date_from={}",
        GOCARDLESS_BASE_URL, gc_account_id, date_from
    );

    debug!("Fetching transactions from GoCardless: {}", url);

    let response = http_client()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch transactions: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Failed to fetch transactions ({}): {}",
            status, body
        ));
    }

    let tx_resp: GcTransactionsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse transactions: {}", e))?;

    let booked = tx_resp.transactions.booked;
    let total = booked.len();
    info!("Fetched {} booked transactions from GoCardless", total);

    if booked.is_empty() {
        return Ok(NordigenSyncResult {
            account_id,
            transactions_imported: 0,
            transactions_skipped: 0,
        });
    }

    // Get account currency
    let wf_account = state
        .account_service()
        .get_account(&account_id)
        .map_err(|e| format!("Failed to get account: {}", e))?;

    let account_currency = wf_account.currency.clone();

    // Map to Wealthfolio universal activity format
    let activities: Vec<wealthfolio_connect::AccountUniversalActivity> = booked
        .into_iter()
        .filter_map(|tx| map_gc_transaction(tx, &account_currency))
        .collect();

    let mapped = activities.len();
    let skipped = total - mapped;

    if activities.is_empty() {
        return Ok(NordigenSyncResult {
            account_id,
            transactions_imported: 0,
            transactions_skipped: skipped,
        });
    }

    // Upsert via existing sync service (handles deduplication)
    let (imported, _assets_created, _new_asset_ids, _needs_review) = state
        .sync_service()
        .upsert_account_activities(account_id.clone(), None, activities)
        .await
        .map_err(|e| format!("Failed to import transactions: {}", e))?;

    info!(
        "Nordigen sync complete: {} imported, {} skipped",
        imported, skipped
    );

    Ok(NordigenSyncResult {
        account_id,
        transactions_imported: imported,
        transactions_skipped: skipped,
    })
}

/// Delete a GoCardless requisition (revoke bank access).
#[tauri::command]
pub async fn nordigen_delete_requisition(requisition_id: String) -> Result<(), String> {
    let token = get_access_token().await?;

    let response = http_client()
        .delete(format!(
            "{}/requisitions/{}/",
            GOCARDLESS_BASE_URL, requisition_id
        ))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Failed to delete requisition: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Failed to delete requisition: {}", body));
    }

    info!("Requisition {} deleted", requisition_id);
    Ok(())
}

/// Remove all GoCardless credentials and tokens from the keyring.
#[tauri::command]
pub async fn nordigen_clear_credentials() -> Result<(), String> {
    let _ = KeyringSecretStore.delete_secret(SECRET_CLIENT_ID);
    let _ = KeyringSecretStore.delete_secret(SECRET_CLIENT_SECRET);
    let _ = KeyringSecretStore.delete_secret(SECRET_ACCESS_TOKEN);
    let _ = KeyringSecretStore.delete_secret(SECRET_REFRESH_TOKEN);
    info!("GoCardless credentials cleared");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Transaction mapping
// ─────────────────────────────────────────────────────────────────────────────

fn map_gc_transaction(
    tx: GcTransaction,
    _account_currency: &str,
) -> Option<wealthfolio_connect::AccountUniversalActivity> {
    let amount_str = tx.transaction_amount.amount.replace(',', ".");
    let amount: f64 = amount_str.parse().ok()?;

    if amount == 0.0 {
        return None;
    }

    let date = tx.booking_date.or(tx.value_date)?;

    // Positive = credit (DEPOSIT), Negative = debit (WITHDRAWAL)
    let (activity_type, abs_amount) = if amount > 0.0 {
        ("DEPOSIT", amount)
    } else {
        ("WITHDRAWAL", amount.abs())
    };

    let description = tx
        .remittance_information_unstructured
        .or(tx.remittance_information_structured)
        .or(tx.creditor_name)
        .or(tx.debtor_name)
        .unwrap_or_default();

    // Use GoCardless transaction ID for deduplication
    let external_id = tx.transaction_id.unwrap_or_else(|| {
        format!(
            "gc-{}-{}",
            date,
            amount_str.replace('.', "_").replace('-', "m")
        )
    });

    Some(wealthfolio_connect::AccountUniversalActivity {
        id: Some(external_id.clone()),
        source_record_id: Some(external_id),
        source_system: Some("GOCARDLESS".to_string()),
        activity_type: Some(activity_type.to_string()),
        trade_date: Some(date),
        settlement_date: None,
        price: Some(abs_amount),
        units: Some(1.0),
        amount: Some(abs_amount),
        currency: Some(wealthfolio_connect::AccountUniversalActivityCurrency {
            id: None,
            code: Some(tx.transaction_amount.currency),
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
    })
}
