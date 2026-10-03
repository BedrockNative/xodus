//! Public Xbox presentation metadata; credentials never leave this module.
use crate::models::{live::ExchangeUserTokenOutcome, secrets::Token, soap};
use crate::tokens::TokenManager;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct XboxProfile {
    pub id: String,
    pub gamertag: String,
    pub picture_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("Xbox profile is unavailable")]
pub struct ProfileError;

/// Uses a snapshot of exactly the requested saved account, without selecting it.
/// No interactive login, device provisioning, license checks or persistent writes.
pub async fn fetch(tokens: &TokenManager, id: &str) -> Result<XboxProfile, ProfileError> {
    tokio::time::timeout(Duration::from_secs(20), fetch_inner(tokens, id))
        .await
        .map_err(|_| ProfileError)?
}

async fn fetch_inner(tokens: &TokenManager, id: &str) -> Result<XboxProfile, ProfileError> {
    let session = tokens.session_for_account(id).map_err(|_| ProfileError)?;
    let Token::Legacy(device) = session.get_device_sts_token().map_err(|_| ProfileError)? else {
        return Err(ProfileError);
    };
    let Token::Legacy(user) = session.get_user_sts_token().map_err(|_| ProfileError)? else {
        return Err(ProfileError);
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ProfileError)?;
    let scope = "user.auth.xboxlive.com";
    let issued = crate::api::live::exchange_user_token(
        &client,
        user,
        "USERNAME".into(),
        device,
        None,
        Some("Silent".into()),
        "{d6d5a677-0872-4ab0-9442-bb792fce85c5}".into(),
        &[(scope.into(), Some(soap::PolicyReference::mbi_ssl()))],
    )
    .await
    .map_err(|_| ProfileError)?;
    let token = match issued {
        ExchangeUserTokenOutcome::Issued(soap::BodyContent::RequestSecurityTokenResponse(
            token,
        )) => *token,
        ExchangeUserTokenOutcome::Issued(
            soap::BodyContent::RequestSecurityTokenResponseCollection(collection),
        ) => collection
            .security_tokens
            .into_iter()
            .find(|t| t.applies_to.endpoint_reference.address == scope)
            .ok_or(ProfileError)?,
        _ => return Err(ProfileError),
    };
    if token.applies_to.endpoint_reference.address != scope {
        return Err(ProfileError);
    }
    let Token::Compact(ticket) = token.into() else {
        return Err(ProfileError);
    };
    let user = super::authenticate_xbox_user(&client, ticket)
        .await
        .map_err(|_| ProfileError)?;
    let xsts = super::request_xsts_token(&client, user.token, "http://xboxlive.com")
        .await
        .map_err(|_| ProfileError)?;
    let hash = xsts.user_hash().ok_or(ProfileError)?;
    let mut authorization =
        reqwest::header::HeaderValue::from_str(&format!("XBL3.0 x={hash};{}", xsts.token))
            .map_err(|_| ProfileError)?;
    authorization.set_sensitive(true);
    let mut response = client.get("https://profile.xboxlive.com/users/me/profile/settings?settings=Gamertag,GameDisplayPicRaw")
        .header(reqwest::header::AUTHORIZATION, authorization).header("x-xbl-contract-version", "2")
        .send().await.map_err(|_| ProfileError)?.error_for_status().map_err(|_| ProfileError)?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ProfileError)? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(ProfileError);
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(id, &bytes)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    profile_users: Vec<ProfileUser>,
}
#[derive(Deserialize)]
struct ProfileUser {
    settings: Vec<Setting>,
}
#[derive(Deserialize)]
struct Setting {
    id: String,
    value: String,
}

fn parse(id: &str, bytes: &[u8]) -> Result<XboxProfile, ProfileError> {
    let response: Response = serde_json::from_slice(bytes).map_err(|_| ProfileError)?;
    if response.profile_users.len() != 1 {
        return Err(ProfileError);
    }
    let settings = &response.profile_users[0].settings;
    let gamertag = settings
        .iter()
        .find(|s| s.id == "Gamertag")
        .map(|s| s.value.trim())
        .ok_or(ProfileError)?;
    if gamertag.is_empty() || gamertag.len() > 128 || gamertag.chars().any(char::is_control) {
        return Err(ProfileError);
    }
    let picture_url = settings
        .iter()
        .find(|s| s.id == "GameDisplayPicRaw")
        .filter(|s| valid_picture_url(&s.value))
        .map(|s| s.value.clone());
    Ok(XboxProfile {
        id: id.into(),
        gamertag: gamertag.into(),
        picture_url,
    })
}

fn valid_picture_url(value: &str) -> bool {
    if value.len() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            host == "xboxlive.com"
                || host.ends_with(".xboxlive.com")
                || host == "xbox.com"
                || host.ends_with(".xbox.com")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_safe_presentation_fields_are_returned() {
        let result = parse("account", br#"{"profileUsers":[{"id":"private-xuid","settings":[{"id":"Gamertag","value":"Player#1234"},{"id":"GameDisplayPicRaw","value":"https://images-eds-ssl.xboxlive.com/image?url=test"},{"id":"RealName","value":"Never exported"}]}]}"#).unwrap();
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(result.gamertag, "Player#1234");
        assert!(result.picture_url.is_some());
        assert!(!json.contains("private-xuid") && !json.contains("Never exported"));
    }
    #[test]
    fn rejects_bad_metadata_and_untrusted_image_hosts() {
        for value in [
            "http://images.xboxlive.com/pic",
            "https://xboxlive.com.evil.test/pic",
            "file:///tmp/pic",
            "https://127.0.0.1/pic",
            "https://user@images.xboxlive.com/pic",
        ] {
            assert!(!valid_picture_url(value));
        }
        for json in [
            r#"{"profileUsers":[]}"#,
            r#"{"profileUsers":[{"settings":[]}]}"#,
            r#"{"profileUsers":[{"settings":[{"id":"Gamertag","value":"\n"}]}]}"#,
        ] {
            assert!(parse("account", json.as_bytes()).is_err());
        }
    }
}
