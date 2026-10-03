//! Purchase checks are separate from content licenses: a subscription can grant
//! a content license without owning the game. Never infer purchase from a key.
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::live::store::{self, StoreCredentials};
use crate::models::secrets::SavedAccount;
use crate::tokens::TokenManager;

const COLLECTIONS_URL: &str =
    "https://collections.mp.microsoft.com/v8.0/collections/b2bLicensePreview";
const MAX_PAGES: usize = 20;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    rename = "OwnershipRequest",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub struct OwnershipRequest {
    pub product_id: String,
}

impl OwnershipRequest {
    pub fn is_valid(&self) -> bool {
        self.product_id.len() == 12
            && self
                .product_id
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipStatus {
    Purchased,
    NotOwned,
    NoAccount,
    Unavailable,
    InvalidRequest,
}

/// Deliberately excludes account identifiers, credentials, licenses and keys.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename = "OwnershipResponse", rename_all = "camelCase")]
pub struct OwnershipResponse {
    pub product_id: String,
    pub status: OwnershipStatus,
}

/// Checks saved accounts in this TokenManager's profile, without downloading
/// content or acquiring a decryption key. Results are intentionally not cached.
pub async fn check(tokens: &TokenManager, request: &OwnershipRequest) -> OwnershipResponse {
    let status = if !request.is_valid() {
        OwnershipStatus::InvalidRequest
    } else {
        match tokio::time::timeout(CHECK_TIMEOUT, check_accounts(tokens, &request.product_id)).await
        {
            Ok(Ok(status)) => status,
            // Never expose upstream errors (which may contain account data).
            _ => OwnershipStatus::Unavailable,
        }
    };
    OwnershipResponse {
        product_id: request.product_id.clone(),
        status,
    }
}

async fn check_accounts(tokens: &TokenManager, product_id: &str) -> Result<OwnershipStatus, ()> {
    let accounts = tokens.ownership_accounts().map_err(|_| ())?;
    if accounts.is_empty() {
        return Ok(OwnershipStatus::NoAccount);
    }
    let mut unavailable = false;
    for account in accounts {
        match check_account(tokens, account, product_id).await {
            Ok(OwnershipStatus::Purchased) => return Ok(OwnershipStatus::Purchased),
            Ok(OwnershipStatus::NotOwned) => {}
            _ => unavailable = true,
        }
    }
    Ok(if unavailable {
        OwnershipStatus::Unavailable
    } else {
        OwnershipStatus::NotOwned
    })
}

async fn check_account(
    tokens: &TokenManager,
    account: SavedAccount,
    product_id: &str,
) -> Result<OwnershipStatus, ()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("xodus/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ())?;
    let credentials = store::authenticate(&client, tokens, account)
        .await
        .map_err(|_| tracing::warn!("Ownership: Microsoft Store authentication failed"))?;
    query_collections(&client, COLLECTIONS_URL, &credentials, product_id).await
}

async fn query_collections(
    client: &reqwest::Client,
    endpoint: &str,
    credentials: &StoreCredentials,
    product_id: &str,
) -> Result<OwnershipStatus, ()> {
    let mut continuation: Option<String> = None;
    let mut seen = HashSet::new();
    let mut evidence = HashMap::new();
    for _ in 0..MAX_PAGES {
        let body = serde_json::json!({
            "beneficiaries": [{"identityType": "Msa", "identityValue": credentials.user_token, "localTicketReference": credentials.ticket_reference}],
            "productSkuIds": [{"productId": product_id}],
            "validityType": "Valid",
            "market": "neutral",
            "maxPageSize": 100,
            "expandSatisfyingItems": true,
            "excludeDuplicates": false,
            "continuationToken": continuation,
        });
        let mut response = client
            .post(endpoint)
            .header("Authorization", &credentials.device_token)
            .json(&body)
            .send()
            .await
            .map_err(|_| ())?;
        // 401/403, throttling and server failures do not mean "not owned".
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Ownership: Collections query failed");
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        let page: CollectionPage = serde_json::from_slice(&bytes).map_err(|_| ())?;
        tracing::debug!(
            items = page.items.len(),
            has_next_page = page.continuation_token.is_some(),
            "Ownership: collection page received"
        );
        for item in page.items {
            tracing::debug!(product = %item.product_id, status = %item.status, acquisition = ?item.acquisition_type, "Ownership: public product entitlement classification");
            if item.product_id != product_id {
                continue;
            }
            let status = item.classify(product_id, Utc::now());
            match evidence.entry(item.id) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(status);
                }
                std::collections::hash_map::Entry::Occupied(mut entry)
                    if *entry.get() != status =>
                {
                    // A duplicate entitlement with contradictory states is not
                    // evidence of a purchase (e.g. a refund on a later page).
                    entry.insert(ItemStatus::Ambiguous);
                }
                _ => {}
            }
        }
        continuation = page.continuation_token.filter(|s| !s.is_empty());
        match &continuation {
            None => {
                return Ok(if evidence.values().any(|s| *s == ItemStatus::Purchased) {
                    OwnershipStatus::Purchased
                } else if evidence.values().any(|s| *s == ItemStatus::Ambiguous) {
                    OwnershipStatus::Unavailable
                } else {
                    OwnershipStatus::NotOwned
                });
            }
            Some(token) if token.len() <= 16384 && seen.insert(token.clone()) => {}
            _ => return Err(()),
        }
    }
    Err(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollectionPage {
    // Required: an arbitrary error object or {} must not look like an empty collection.
    items: Vec<CollectionItem>,
    continuation_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollectionItem {
    id: String,
    product_id: String,
    status: String,
    acquisition_type: Option<String>,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
    trial_data: Option<TrialData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrialData {
    is_trial: Option<bool>,
    is_in_trial_period: Option<bool>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ItemStatus {
    Purchased,
    Other,
    Ambiguous,
}

impl CollectionItem {
    fn classify(&self, product_id: &str, now: DateTime<Utc>) -> ItemStatus {
        if self.product_id != product_id || self.status != "Active" {
            return ItemStatus::Other;
        }
        match &self.trial_data {
            Some(t) if t.is_trial == Some(true) || t.is_in_trial_period == Some(true) => {
                return ItemStatus::Other;
            }
            Some(t) if t.is_trial == Some(false) && t.is_in_trial_period == Some(false) => {}
            _ => return ItemStatus::Ambiguous,
        }
        match self.acquisition_type.as_deref() {
            Some("Recurring" | "Conditional") => return ItemStatus::Other,
            Some("Single") => {}
            _ => return ItemStatus::Ambiguous,
        }
        let (Some(start), Some(end)) = (self.start_date, self.end_date) else {
            return ItemStatus::Ambiguous;
        };
        if start > now || end <= now {
            return ItemStatus::Other;
        }
        // Microsoft defines Single as a purchase or redeemed code; unlike
        // licenseToken's endDate/isShared fields, this establishes acquisition.
        ItemStatus::Purchased
    }
}

#[cfg(test)]
mod tests;
