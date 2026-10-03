//! Microsoft Store authentication shared by ownership and content licensing.
use crate::models::{
    live::ExchangeUserTokenOutcome,
    secrets::{SavedAccount, Token},
    soap,
};
use crate::tokens::TokenManager;

const SCOPE: &str = "www.microsoft.com";
const HOSTING_APP: &str = "{d6d5a677-0872-4ab0-9442-bb792fce85c5}";

// Intentionally not Debug/Serialize: these values must never enter logs or IPC.
pub struct StoreCredentials {
    pub device_token: String,
    pub user_token: String,
    pub ticket_reference: String,
}

#[derive(Debug, thiserror::Error)]
#[error("Microsoft Store authentication is unavailable")]
pub struct AuthenticationError;

pub async fn authenticate(
    client: &reqwest::Client,
    tokens: &TokenManager,
    account: SavedAccount,
) -> Result<StoreCredentials, AuthenticationError> {
    let Token::Legacy(device_token) = tokens
        .get_device_sts_token()
        .map_err(|_| AuthenticationError)?
    else {
        return Err(AuthenticationError);
    };
    let Token::Legacy(user_token) = account.sts else {
        return Err(AuthenticationError);
    };
    let device = super::exchange_device_token(
        client,
        device_token.clone(),
        HOSTING_APP.into(),
        SCOPE.into(),
        Some(soap::PolicyReference::mbi_ssl()),
    )
    .await
    .map_err(|_| AuthenticationError)?;
    let issued = super::exchange_user_token(
        client,
        user_token,
        account.user.username,
        device_token,
        None,
        Some("Silent".into()),
        HOSTING_APP.into(),
        &[(SCOPE.into(), Some(soap::PolicyReference::mbi_ssl()))],
    )
    .await
    .map_err(|_| AuthenticationError)?;
    let user = match issued {
        ExchangeUserTokenOutcome::Issued(soap::BodyContent::RequestSecurityTokenResponse(
            token,
        )) => *token,
        ExchangeUserTokenOutcome::Issued(
            soap::BodyContent::RequestSecurityTokenResponseCollection(collection),
        ) => collection
            .security_tokens
            .into_iter()
            .find(|t| t.applies_to.endpoint_reference.address == SCOPE)
            .ok_or(AuthenticationError)?,
        _ => return Err(AuthenticationError),
    };
    Ok(StoreCredentials {
        device_token: compact_token(device)?,
        user_token: compact_token(user)?,
        ticket_reference: account.user.puid,
    })
}

fn compact_token(token: soap::RequestSecurityTokenResponse) -> Result<String, AuthenticationError> {
    if token.applies_to.endpoint_reference.address != SCOPE {
        return Err(AuthenticationError);
    }
    let value = token
        .requested_security_token
        .binary_security_token
        .ok_or(AuthenticationError)?
        .value;
    if value.is_empty() {
        return Err(AuthenticationError);
    }
    match token.token_type.as_str() {
        "urn:passport:compact" => Ok(value),
        "urn:passport:delegationcompact" => Ok(format!("d={value}")),
        _ => Err(AuthenticationError),
    }
}
