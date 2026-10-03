use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MSATokenRequest {
    pub client_id: String,
    #[serde(default)]
    pub allow_ui: bool,
    #[serde(default, alias = "MSAFullTrust")]
    pub msa_full_trust: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MSATokenResponse {
    pub token: String,
    pub expiry: i64,
    pub device_rps: String,
    pub device_expiry: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub puid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
}

/// Desktop WinRT broker request. Application identity is supplied by the host.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct XboxTokenRequest {
    pub client_id: String,
    pub title_id: i64,
    /// Resource URL, resolved through the Xbox endpoint metadata.
    pub relying_party: String,
    #[serde(default)]
    pub http_method: String,
    #[serde(default)]
    pub request_headers: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct XboxTokenResponse {
    pub token: String,
    pub expiry: i64,
    pub puid: String,
    pub user_name: String,
    pub xuid: String,
    pub gamertag: String,
    pub user_hash: String,
    pub signature: String,
    pub sandbox: String,
    pub age_group: String,
    pub modern_gamertag: String,
    pub modern_gamertag_suffix: String,
    pub unique_modern_gamertag: String,
    pub privileges: String,
    pub enforcement: String,
    pub restrictions: String,
    pub title_restrictions: String,
}
