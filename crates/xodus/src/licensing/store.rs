//! Legacy UWP Store receipts and licensing, obtained from Microsoft for the
//! signed-in account. Receipt XML is returned unchanged; it is never generated.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{live::ExchangeUserTokenOutcome, secrets::Token, soap};
use crate::tokens::TokenManager;

const E_FAIL: u32 = 0x80004005;
const E_INVALIDARG: u32 = 0x80070057;
const E_ACCESSDENIED: u32 = 0x80070005;
const NOT_FOUND: u32 = 0x80070490;
const NOT_SUPPORTED: u32 = 0x80004001;
const MAX_REPLY: usize = 1 << 20;

#[derive(Deserialize)]
#[serde(rename = "StoreRequest", rename_all = "PascalCase")]
pub struct StoreRequest {
    pub package_family_name: String,
    pub market: String,
    pub language: String,
    pub operation: String,
    #[serde(default)]
    pub product_id: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename = "StoreResponse", rename_all = "PascalCase")]
pub struct StoreResponse {
    pub status: u32,
    pub app_id: String,
    pub store_id: String,
    pub package_family_name: String,
    pub receipt: String,
    pub is_active: bool,
    pub is_trial: bool,
    pub expiration_date: i64,
    pub name: String,
    pub description: String,
    pub formatted_price: String,
    pub currency: String,
    pub age_rating: u32,
    pub current_market: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("invalid Store request")]
    InvalidRequest,
    #[error("Store credentials unavailable")]
    Credentials,
    #[error("Store authentication failed")]
    Authentication,
    #[error("Store catalog has no matching package")]
    NotFound,
    #[error("unsupported Store operation")]
    Unsupported,
    #[error("Store service returned HTTP {0}")]
    Http(u16),
    #[error("Store service returned an invalid response ({0})")]
    InvalidResponse(&'static str),
    #[error("Store transport failed")]
    Transport,
}
impl StoreError {
    fn hresult(&self) -> u32 {
        match self {
            Self::InvalidRequest => E_INVALIDARG,
            Self::Credentials | Self::Authentication => E_ACCESSDENIED,
            Self::NotFound => NOT_FOUND,
            Self::Unsupported => NOT_SUPPORTED,
            Self::Http(code) => 0x80190000 | u32::from(*code),
            _ => E_FAIL,
        }
    }
}

type CacheKey = (String, String, String, String, String);
type Slot = Arc<tokio::sync::Mutex<Option<(Instant, StoreResponse)>>>;
fn cache_slot(key: CacheKey) -> Slot {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Slot>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if cache.len() >= 16 && !cache.contains_key(&key) {
        cache.clear();
    }
    cache.entry(key).or_default().clone()
}

pub async fn handle(tokens: &TokenManager, request: StoreRequest) -> StoreResponse {
    match request_store(tokens, &request).await {
        Ok(reply) => reply,
        Err(error) => {
            // Error variants contain no credentials, response bodies or account IDs.
            tracing::warn!("{error}");
            StoreResponse {
                status: error.hresult(),
                ..Default::default()
            }
        }
    }
}
fn validate_request(request: &StoreRequest) -> Result<(), StoreError> {
    let pfn = &request.package_family_name;
    if pfn.len() > 255
        || pfn.is_empty()
        || !pfn.contains('_')
        || !pfn
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
        || request.market.len() != 2
        || !request.market.bytes().all(|c| c.is_ascii_alphabetic())
        || request.language.is_empty()
        || request.language.len() > 85
        || !request
            .language
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(StoreError::InvalidRequest);
    }
    if request.operation != "AppReceipt"
        && request.operation != "License"
        && request.operation != "Listing"
    {
        return Err(StoreError::Unsupported);
    }
    if !request.product_id.is_empty() {
        return Err(StoreError::Unsupported);
    }
    Ok(())
}
async fn json_reply(response: reqwest::Response) -> Result<Value, StoreError> {
    if !response.status().is_success() {
        return Err(StoreError::Http(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_REPLY as u64)
    {
        return Err(StoreError::InvalidResponse("response"));
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| StoreError::Transport)? {
        if chunk.len() > MAX_REPLY - bytes.len() {
            return Err(StoreError::InvalidResponse("response"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| StoreError::InvalidResponse("response"))
}

async fn store_tokens(
    client: &reqwest::Client,
    tokens: &TokenManager,
) -> Result<(String, String, String), StoreError> {
    let Token::Legacy(device) = tokens
        .get_device_sts_token()
        .map_err(|_| StoreError::Credentials)?
    else {
        return Err(StoreError::Credentials);
    };
    let Token::Legacy(user_token) = tokens
        .get_user_sts_token()
        .map_err(|_| StoreError::Credentials)?
    else {
        return Err(StoreError::Credentials);
    };
    let user = tokens.get_user().map_err(|_| StoreError::Credentials)?;
    let hosting = "{d6d5a677-0872-4ab0-9442-bb792fce85c5}";
    let response = crate::api::live::exchange_device_token(
        client,
        device.clone(),
        hosting.into(),
        "www.microsoft.com".into(),
        Some(soap::PolicyReference::mbi_ssl()),
    )
    .await
    .map_err(|_| StoreError::Authentication)?;
    let Token::Compact(device_ms) = Token::from(response) else {
        return Err(StoreError::Authentication);
    };
    let response = crate::api::live::exchange_user_token(
        client,
        user_token,
        user.username,
        device,
        None,
        Some("Silent".into()),
        hosting.into(),
        &[(
            "www.microsoft.com".into(),
            Some(soap::PolicyReference::mbi_ssl()),
        )],
    )
    .await
    .map_err(|_| StoreError::Authentication)?;
    let token = match response {
        ExchangeUserTokenOutcome::Issued(
            soap::BodyContent::RequestSecurityTokenResponseCollection(mut c),
        ) if !c.security_tokens.is_empty() => c.security_tokens.remove(0),
        ExchangeUserTokenOutcome::Issued(soap::BodyContent::RequestSecurityTokenResponse(t)) => *t,
        _ => return Err(StoreError::Authentication),
    };
    let Token::Compact(user_ms) = Token::from(token) else {
        return Err(StoreError::Authentication);
    };
    Ok((device_ms, user_ms, user.puid))
}

#[derive(Deserialize)]
struct Receipt {
    #[serde(rename = "@CertificateId")]
    certificate: String,
    #[serde(rename = "AppReceipt", default)]
    applications: Vec<AppReceipt>,
    #[serde(rename = "Signature")]
    signature: ReceiptSignature,
}
#[derive(Deserialize)]
struct AppReceipt {
    #[serde(rename = "@AppId")]
    app_id: String,
    #[serde(rename = "@LicenseType")]
    license_type: String,
}
#[derive(Deserialize)]
struct ReceiptSignature {
    #[serde(rename = "SignatureValue")]
    value: String,
}
fn receipt_license(xml: &str, pfn: &str) -> Result<Option<bool>, StoreError> {
    // The bytes come directly from the authenticated Microsoft HTTPS response.
    // The game receives the original signature and can verify it with Microsoft's certificate.
    if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
        return Err(StoreError::InvalidResponse("response"));
    }
    let receipt: Receipt = quick_xml::de::from_str(xml.trim_start_matches('\u{feff}'))
        .map_err(|_| StoreError::InvalidResponse("receipt XML"))?;
    if receipt.certificate.is_empty()
        || receipt.signature.value.is_empty()
        || receipt.applications.len() > 1
    {
        return Err(StoreError::InvalidResponse("receipt signature"));
    }
    match receipt.applications.first() {
        None => Ok(None),
        Some(app) if app.app_id.eq_ignore_ascii_case(pfn) => match app.license_type.as_str() {
            "Full" => Ok(Some(false)),
            "Trial" => Ok(Some(true)),
            _ => Err(StoreError::InvalidResponse("response")),
        },
        _ => Err(StoreError::InvalidResponse("receipt package")),
    }
}
fn matching_product<'a>(catalog: &'a Value, pfn: &str) -> Option<&'a Value> {
    catalog.get("Products")?.as_array()?.iter().find(|product| {
        product
            .get("DisplaySkuAvailabilities")
            .and_then(Value::as_array)
            .is_some_and(|skus| {
                skus.iter().any(|sku| {
                    sku.pointer("/Sku/Properties/Packages")
                        .and_then(Value::as_array)
                        .is_some_and(|packages| {
                            packages.iter().any(|p| {
                                p.get("PackageFamilyName")
                                    .and_then(Value::as_str)
                                    .is_some_and(|name| name.eq_ignore_ascii_case(pfn))
                            })
                        })
                })
            })
    })
}
async fn request_store(
    tokens: &TokenManager,
    request: &StoreRequest,
) -> Result<StoreResponse, StoreError> {
    validate_request(request)?;
    let user = tokens.get_user().map_err(|_| StoreError::Credentials)?;
    // Account removal must invalidate access even during the cache lifetime.
    tokens
        .get_user_sts_token()
        .map_err(|_| StoreError::Credentials)?;
    let slot = cache_slot((
        user.puid.clone(),
        request.package_family_name.clone(),
        request.market.clone(),
        request.language.clone(),
        request.operation.clone(),
    ));
    let mut cached = slot.lock().await;
    if let Some((created, reply)) = cached
        .as_ref()
        .filter(|(time, _)| time.elapsed() < Duration::from_secs(60))
    {
        let _ = created;
        return Ok(reply.clone());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| StoreError::Transport)?;
    let mut url =
        reqwest::Url::parse("https://displaycatalog.mp.microsoft.com/v7.0/products/lookup")
            .map_err(|_| StoreError::InvalidResponse("response"))?;
    url.query_pairs_mut().extend_pairs([
        ("alternateId", "PackageFamilyName"),
        ("value", request.package_family_name.as_str()),
        ("market", request.market.as_str()),
        ("languages", request.language.as_str()),
    ]);
    let catalog = json_reply(
        client
            .get(url)
            .send()
            .await
            .map_err(|_| StoreError::Transport)?,
    )
    .await?;
    // Lookup returns a compact listing; fetch complete package metadata before
    // accepting a product, so a similarly named catalog entry cannot be used.
    let ids = catalog
        .get("BigIds")
        .and_then(Value::as_array)
        .ok_or(StoreError::NotFound)?;
    let mut detailed = None;
    for id in ids.iter().take(16).filter_map(Value::as_str) {
        if id.len() != 12 || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let mut url = reqwest::Url::parse(&format!(
            "https://displaycatalog.mp.microsoft.com/v7.0/products/{id}"
        ))
        .map_err(|_| StoreError::InvalidResponse("response"))?;
        url.query_pairs_mut().extend_pairs([
            ("market", request.market.as_str()),
            ("languages", request.language.as_str()),
        ]);
        let full = json_reply(
            client
                .get(url)
                .send()
                .await
                .map_err(|_| StoreError::Transport)?,
        )
        .await?;
        let full = serde_json::json!({"Products": [full.get("Product").ok_or(StoreError::InvalidResponse("response"))?]});
        if matching_product(&full, &request.package_family_name).is_some() {
            detailed = Some(full);
            break;
        }
    }
    let catalog = detailed.ok_or(StoreError::NotFound)?;
    let product =
        matching_product(&catalog, &request.package_family_name).ok_or(StoreError::NotFound)?;
    let store_id = product
        .get("ProductId")
        .and_then(Value::as_str)
        .ok_or(StoreError::InvalidResponse("response"))?;
    let app_id = product
        .get("AlternateIds")
        .and_then(Value::as_array)
        .and_then(|ids| {
            ids.iter().find(|id| {
                id.get("IdType").and_then(Value::as_str) == Some("LegacyWindowsStoreProductId")
            })
        })
        .and_then(|id| id.get("Value"))
        .and_then(Value::as_str)
        .ok_or(StoreError::NotFound)?;
    if uuid::Uuid::parse_str(app_id).is_err() {
        return Err(StoreError::InvalidResponse("response"));
    }
    let (device_ms, user_ms, puid) = store_tokens(&client, tokens).await?;
    if puid != user.puid {
        return Err(StoreError::Credentials);
    }
    let mut url = reqwest::Url::parse(
        "https://licensingwindows.mp.microsoft.com/Licensing/License/AcquireReceipt/6.2/0",
    )
    .map_err(|_| StoreError::InvalidResponse("response"))?;
    url.query_pairs_mut()
        .extend_pairs([("productId", app_id), ("receiptType", "0")]);
    let response = json_reply(
        client
            .post(url)
            .header("Authorization", &user_ms)
            .header("X-Device-Token", &device_ms)
            .send()
            .await
            .map_err(|_| StoreError::Transport)?,
    )
    .await?;
    let error = response
        .get("errorCode")
        .and_then(Value::as_str)
        .ok_or(StoreError::InvalidResponse("response"))?;
    if error != "0x0" && error != "0" {
        return Err(StoreError::InvalidResponse("receipt error code"));
    }
    let receipt = response
        .get("receipt")
        .and_then(Value::as_str)
        .ok_or(StoreError::InvalidResponse("response"))?;
    let receipt = BASE64_STANDARD
        .decode(receipt)
        .map_err(|_| StoreError::InvalidResponse("response"))?;
    let receipt =
        String::from_utf8(receipt).map_err(|_| StoreError::InvalidResponse("response"))?;
    let trial = receipt_license(&receipt, &request.package_family_name)?;
    let mut reply = StoreResponse {
        app_id: app_id.into(),
        store_id: store_id.into(),
        package_family_name: request.package_family_name.clone(),
        receipt,
        current_market: request.market.clone(),
        is_trial: trial == Some(true),
        ..Default::default()
    };
    if let Some(localized) = product
        .get("LocalizedProperties")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
    {
        reply.name = localized
            .get("ProductTitle")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into();
        reply.description = localized
            .get("ShortDescription")
            .or_else(|| localized.get("ProductDescription"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(4096)
            .collect();
    }
    let preferred = product.get("PreferredSkuId").and_then(Value::as_str);
    let skus = product
        .get("DisplaySkuAvailabilities")
        .and_then(Value::as_array)
        .ok_or(StoreError::InvalidResponse("response"))?;
    let sku = skus
        .iter()
        .find(|s| s.pointer("/Sku/SkuId").and_then(Value::as_str) == preferred)
        .or_else(|| skus.first())
        .ok_or(StoreError::NotFound)?;
    if let Some(price) = sku
        .get("Availabilities")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|a| a.pointer("/OrderManagementData/Price"))
    {
        reply.currency = price
            .get("CurrencyCode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into();
        if let Some(value) = price.get("ListPrice").and_then(Value::as_f64) {
            reply.formatted_price = format!("{} {:.2}", reply.currency, value);
        }
    }
    if trial.is_some() && request.operation == "License" {
        let content_id = skus
            .iter()
            .filter_map(|s| {
                s.pointer("/Sku/Properties/Packages")
                    .and_then(Value::as_array)
            })
            .flatten()
            .find(|p| {
                p.get("PackageFamilyName")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(&request.package_family_name))
            })
            .and_then(|p| p.get("ContentId"))
            .and_then(Value::as_str)
            .ok_or(StoreError::NotFound)?;
        match super::content::get_license_information(
            &client,
            device_ms,
            user_ms,
            puid,
            content_id.into(),
            request.market.clone(),
        )
        .await
        {
            Ok((content, license)) => {
                let (active, expiration) = runtime_license_state(
                    &license,
                    &content,
                    &request.package_family_name,
                    chrono::Utc::now().timestamp(),
                )?;
                reply.is_active = active;
                reply.is_trial = false;
                reply.expiration_date = expiration;
            }
            Err(super::content::LicenseContentError::NotEntitled { .. }) => reply.is_active = false,
            Err(super::content::LicenseContentError::Request(error)) => {
                return Err(if let Some(status) = error.status() {
                    StoreError::Http(status.as_u16())
                } else if error.is_decode() {
                    StoreError::InvalidResponse("runtime lease JSON")
                } else {
                    StoreError::Transport
                });
            }
            Err(super::content::LicenseContentError::InvalidResponse(reason)) => {
                return Err(StoreError::InvalidResponse(reason));
            }
        }
    }
    *cached = Some((Instant::now(), reply.clone()));
    Ok(reply)
}

// A content-key license establishes entitlement; a lease alone does not.
// Only fresh online leases are accepted here. Offline lease renewal semantics
// are deliberately not inferred from the renewal-period field.
fn runtime_license_state(
    license: &crate::models::devicecredential::License,
    content: &crate::models::licensing::LicenseContent,
    pfn: &str,
    now: i64,
) -> Result<(bool, i64), StoreError> {
    use crate::models::devicecredential::{License, LicenseType};
    if !matches!(license.license_info.license_type, LicenseType::Full) {
        return Err(StoreError::Unsupported);
    }
    let decoded = super::splicense::SPLicense::parse_base64(&license.splicense_block)
        .map_err(|_| StoreError::InvalidResponse("content license parse"))?;
    if !decoded
        .package_name
        .trim_end_matches('\0')
        .eq_ignore_ascii_case(pfn)
    {
        return Err(StoreError::InvalidResponse("content license package"));
    }
    let mut expiry = i64::from(decoded.license_expiration_time);
    let mut expired = decoded.basic_policies & 4 != 0;
    if decoded.basic_policies & 1 != 0
        || license
            .binding
            .lease_required
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        let mut valid_lease = false;
        for entry in &content.leases {
            let xml = BASE64_STANDARD
                .decode(&entry.value)
                .map_err(|_| StoreError::InvalidResponse("lease base64"))?;
            let lease: License = quick_xml::de::from_reader(xml.as_slice())
                .map_err(|_| StoreError::InvalidResponse("lease XML"))?;
            let decoded_lease = super::splicense::SPLicense::parse_base64(&lease.splicense_block)
                .map_err(|_| StoreError::InvalidResponse("lease parse"))?;
            let issued = lease
                .license_info
                .issued_date
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp());
            if matches!(lease.license_info.license_type, LicenseType::Lease)
                && decoded_lease
                    .package_name
                    .trim_end_matches('\0')
                    .eq_ignore_ascii_case(pfn)
                && issued.is_some_and(|t| t <= now + 60 && t >= now - 300)
                && (decoded_lease.license_expiration_time == 0
                    || i64::from(decoded_lease.license_expiration_time) > now)
            {
                valid_lease = true;
                // The parent unlock license requires the accompanying runtime
                // lease. Its key expiration is not the application's expiry.
                expiry = i64::from(decoded_lease.license_expiration_time);
                expired |= decoded_lease.basic_policies & 4 != 0;
                break;
            }
        }
        if !valid_lease {
            return Err(StoreError::InvalidResponse("fresh runtime lease missing"));
        }
    }
    let begin = license
        .license_info
        .begin_date
        .as_deref()
        .map(chrono::DateTime::parse_from_rfc3339)
        .transpose()
        .map_err(|_| StoreError::InvalidResponse("license begin date"))?
        .map(|t| t.timestamp());
    // Full content licenses without an absolute expiry are non-expiring.
    // Store's legacy receipt can still describe an earlier trial purchase.
    let expiration = if expiry == 0 {
        2_650_467_743_999_999_999
    } else {
        (expiry + 11_644_473_600) * 10_000_000
    };
    Ok((
        !expired && begin.is_none_or(|t| t <= now) && (expiry == 0 || expiry > now),
        expiration,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn license_fixture(
        kind: &str,
        package: &str,
        expiry: u32,
        policies: u16,
        lease_required: bool,
        issued: &str,
    ) -> crate::models::devicecredential::License {
        let mut block = vec![0u8; 8];
        let package: Vec<u8> = package.encode_utf16().flat_map(u16::to_le_bytes).collect();
        for (id, data) in [
            (0xceu32, package),
            (0x20, expiry.to_le_bytes().to_vec()),
            (0xc9, {
                let mut v = vec![0u8; 8];
                v.extend(policies.to_le_bytes());
                v
            }),
        ] {
            block.extend(id.to_le_bytes());
            block.extend((data.len() as u32).to_le_bytes());
            block.extend(data);
        }
        let xml = format!(
            r#"<License><SPLicenseBlock>{}</SPLicenseBlock><LicenseInfo Type="{kind}" LicenseUsage="Online"><IssuedDate>{issued}</IssuedDate></LicenseInfo><Binding Binding_Type="Device"><LeaseRequired>{lease_required}</LeaseRequired></Binding></License>"#,
            BASE64_STANDARD.encode(block)
        );
        quick_xml::de::from_str(&xml).unwrap()
    }

    #[test]
    fn store_runtime_license_requires_entitlement_and_fresh_matching_lease() {
        use crate::models::licensing::{LicenseContent, LicenseKeys};
        let now = 1_800_000_000;
        let issued = chrono::DateTime::from_timestamp(now, 0)
            .unwrap()
            .to_rfc3339();
        let pfn = "Example.App_abc";
        let mut content = LicenseContent {
            keys: vec![],
            leases: vec![],
        };
        let full = license_fixture("Full", pfn, now as u32, 11, true, &issued);
        assert!(runtime_license_state(&full, &content, pfn, now).is_err());
        let lease = license_fixture("Lease", pfn, 0, 0, false, &issued);
        assert!(runtime_license_state(&lease, &content, pfn, now).is_err());
        let xml = format!(
            r#"<License><SPLicenseBlock>{}</SPLicenseBlock><LicenseInfo Type="Lease"><IssuedDate>{issued}</IssuedDate></LicenseInfo><Binding Binding_Type="Device"/></License>"#,
            lease.splicense_block
        );
        content.leases.push(LicenseKeys {
            value: BASE64_STANDARD.encode(xml),
        });
        assert!(runtime_license_state(&full, &content, pfn, now).unwrap().0);
        assert!(runtime_license_state(&full, &content, "Other.App_abc", now).is_err());
        assert!(runtime_license_state(&full, &content, pfn, now + 301).is_err());
        let expired = license_fixture("Full", pfn, 0, 4, false, &issued);
        assert!(
            !runtime_license_state(&expired, &content, pfn, now)
                .unwrap()
                .0
        );
        let timed = license_fixture("Full", pfn, (now - 1) as u32, 0, false, &issued);
        assert!(!runtime_license_state(&timed, &content, pfn, now).unwrap().0);
    }

    #[test]
    fn store_request_rejects_paths_and_foreign_operations() {
        let mut r = StoreRequest {
            package_family_name: "Example.App_123456789abcd".into(),
            market: "US".into(),
            language: "en-US".into(),
            operation: "AppReceipt".into(),
            product_id: String::new(),
        };
        assert!(validate_request(&r).is_ok());
        r.package_family_name = "../other_app".into();
        assert!(validate_request(&r).is_err());
        r.package_family_name = "Example.App_123456789abcd".into();
        r.operation = "Purchase".into();
        assert!(matches!(validate_request(&r), Err(StoreError::Unsupported)));
    }
    #[test]
    fn store_cache_separates_accounts_and_packages() {
        let key = (
            "account-a".into(),
            "app-a".into(),
            "US".into(),
            "en-US".into(),
            "AppReceipt".into(),
        );
        let a = cache_slot(key.clone());
        assert!(Arc::ptr_eq(&a, &cache_slot(key)));
        assert!(!Arc::ptr_eq(
            &a,
            &cache_slot((
                "account-b".into(),
                "app-a".into(),
                "US".into(),
                "en-US".into(),
                "AppReceipt".into()
            ))
        ));
        assert!(!Arc::ptr_eq(
            &a,
            &cache_slot((
                "account-a".into(),
                "app-b".into(),
                "US".into(),
                "en-US".into(),
                "AppReceipt".into()
            ))
        ));
    }
    #[test]
    fn store_catalog_requires_matching_package() {
        let catalog = serde_json::json!({"Products":[{"ProductId":"wrong","DisplaySkuAvailabilities":[{"Sku":{"Properties":{"Packages":[{"PackageFamilyName":"Other.App_abc"}]}}}]}]});
        assert!(matching_product(&catalog, "Expected.App_abc").is_none());
        assert!(matching_product(&catalog, "Other.App_abc").is_some());
    }
    #[test]
    fn store_receipt_rejects_unsigned_and_wrong_app() {
        assert!(receipt_license("<Receipt/>", "Expected.App_abc").is_err());
        let xml = "<Receipt CertificateId='test'><AppReceipt AppId='Other.App_abc' LicenseType='Full'/><Signature><SignatureValue>test</SignatureValue></Signature></Receipt>";
        assert!(receipt_license(xml, "Expected.App_abc").is_err());
    }
}
