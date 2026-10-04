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
    pub gdk_session_cache_version: u32,
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
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub package_family_name: String,
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

impl XboxTokenRequest {
    /// Older UWP brokers resolve a public application registration from package
    /// identity. This entry is shared by the legacy and current Windows package;
    /// it grants no entitlement and authentication still goes through Xbox.
    pub fn resolved_client_id(&self) -> Result<&str, &'static str> {
        if self.title_id <= 0 {
            return Err("Application title identity is required");
        }
        if !self.client_id.is_empty() {
            return Ok(&self.client_id);
        }
        match (self.title_id, self.package_family_name.as_str()) {
            (896928775, "Microsoft.MinecraftUWP_8wekyb3d8bbwe") => Ok("0000000040159362"),
            _ => Err("No public OAuth registration for this package and title"),
        }
    }
}
#[cfg(test)]
mod registration_tests {
    use super::*;
    fn request(package: &str, title: i64, client: &str) -> XboxTokenRequest {
        XboxTokenRequest {
            client_id: client.into(),
            package_family_name: package.into(),
            title_id: title,
            relying_party: "https://xboxlive.com".into(),
            http_method: String::new(),
            request_headers: String::new(),
        }
    }
    #[test]
    fn legacy_registration_requires_matching_package_and_title() {
        assert_eq!(
            request("Microsoft.MinecraftUWP_8wekyb3d8bbwe", 896928775, "").resolved_client_id(),
            Ok("0000000040159362")
        );
        assert!(
            request("Other.Package_8wekyb3d8bbwe", 896928775, "")
                .resolved_client_id()
                .is_err()
        );
        assert!(
            request("Microsoft.MinecraftUWP_8wekyb3d8bbwe", 1, "")
                .resolved_client_id()
                .is_err()
        );
        assert!(request("", 896928775, "").resolved_client_id().is_err());
        assert_eq!(
            request("", 123, "explicit-client").resolved_client_id(),
            Ok("explicit-client")
        );
        assert!(
            request("", 0, "explicit-client")
                .resolved_client_id()
                .is_err()
        );
    }
    #[test]
    fn previous_protocol_request_remains_supported() {
        let req: XboxTokenRequest = quick_xml::de::from_str("<XboxTokenRequest><ClientId>explicit-client</ClientId><TitleId>123</TitleId><RelyingParty>https://xboxlive.com</RelyingParty></XboxTokenRequest>").unwrap();
        assert_eq!(req.resolved_client_id(), Ok("explicit-client"));
    }
}
