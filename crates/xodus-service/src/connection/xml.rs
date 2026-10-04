use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xodus::models::live::ExchangeUserTokenOutcome;
use xodus::models::secrets::Token;
use xodus::models::soap;
use xodus::models::xgameruntime::xuser::{
    MSATokenRequest, MSATokenResponse, XboxTokenRequest, XboxTokenResponse,
};
use xodus::proto::xodus::XodusMessageType;

use crate::XML_MAGIC;
use crate::simple_context::SimpleContext;

pub async fn handle(
    socket: &mut tokio::net::UnixStream,
    context: &mut SimpleContext,
) -> tokio::io::Result<()> {
    tracing::debug!("Parsing XML");
    let message_type = socket.read_u16_le().await?;
    let message_size = socket.read_u16_le().await?;
    let mut buffer = vec![0; message_size as usize];
    tracing::debug!("Reading buffer {message_size}");
    socket.read_exact(&mut buffer).await?;
    tracing::debug!("Read buffer");
    let message_type = XodusMessageType::try_from(message_type as i32).unwrap_or_default();

    let out_buf = match parse_message(context, message_type, buffer).await {
        Ok(buf) => buf,
        Err(err) => {
            tracing::error!("Failed parsing message: {err}");
            vec![]
        }
    };

    let data = xodus::ipc::encode_message(XML_MAGIC, message_type as u16 + 1, &out_buf)?;
    socket.write_all(&data).await
}

pub async fn parse_message(
    context: &mut SimpleContext,
    message_type: XodusMessageType,
    buffer: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    match message_type {
        XodusMessageType::Ping => Ok(buffer),
        XodusMessageType::GdkSessionRequest => super::gdk_session::handle(context, &buffer),
        XodusMessageType::OwnershipRequest => {
            use xodus::licensing::ownership::{
                self, OwnershipRequest, OwnershipResponse, OwnershipStatus,
            };
            let response =
                match quick_xml::de::from_reader::<_, OwnershipRequest>(buffer.as_slice()) {
                    Ok(request) => ownership::check(context.tokens(), &request).await,
                    Err(_) => OwnershipResponse {
                        product_id: String::new(),
                        status: OwnershipStatus::InvalidRequest,
                    },
                };
            Ok(quick_xml::se::to_string(&response)?.into_bytes())
        }
        XodusMessageType::MsaTokenRequest => {
            let string_buf = std::str::from_utf8(&buffer)?;
            let req = quick_xml::de::from_str::<MSATokenRequest>(string_buf)?;
            let Token::Legacy(token) = context.tokens().get_user_sts_token()? else {
                return Ok(vec![]);
            };
            let scope = if req.msa_full_trust {
                "service::user.auth.xboxlive.com::MBI_SSL"
            } else {
                "xboxlive.signin"
            };
            let device_token = context
                .device_token
                .as_ref()
                .ok_or("Device credentials unavailable")?;
            let user = context.tokens().get_user().ok();
            let device_token_resp = xodus::api::live::exchange_device_token(
                &context.client,
                device_token.clone(),
                "{28C08266-F973-4AE6-FFE4-409B249F138F}".to_string(),
                "scope=service::user.auth.xboxlive.com::MBI_SSL".to_owned(),
                Some(soap::PolicyReference::token_broker()),
            )
            .await;

            let ms_device_rps_token = if let Some((Token::Compact(ms_device_token), Ok(lifetime))) =
                device_token_resp.ok().map(|t| {
                    let expiry = chrono::DateTime::parse_from_rfc3339(&t.lifetime.expires);
                    (t.into(), expiry)
                }) {
                Some((ms_device_token, lifetime.timestamp()))
            } else {
                None
            };

            let user_token = xodus::api::live::exchange_user_token(
                &context.client,
                token,
                user.as_ref()
                    .map(|u| u.username.clone())
                    .unwrap_or_default(),
                device_token.clone(),
                None,
                Some("Silent".to_string()),
                req.client_id.clone(),
                &[
                    (
                        format!("scope={scope}&api-version=2.0&clientid={}", req.client_id),
                        Some(soap::PolicyReference::token_broker()),
                    ),
                    ("http://Passport.NET/tb".to_string(), None),
                ],
            )
            .await?;

            match user_token {
                ExchangeUserTokenOutcome::Issued(
                    soap::BodyContent::RequestSecurityTokenResponseCollection(mut collection),
                ) => {
                    if collection.security_tokens.len() < 2 {
                        return Err("Incomplete MSA token response".into());
                    }
                    if let Some(sts) = collection.security_tokens.pop() {
                        let address = sts.applies_to.endpoint_reference.address.clone();
                        let sts: Token = sts.into();
                        let address = if let Token::Legacy(legacy) = &sts {
                            legacy.key_name.clone().unwrap_or(address)
                        } else {
                            address
                        };
                        if let Err(err) = context.tokens().save_user_token(address, sts) {
                            tracing::warn!("Failed to persist refreshed STS token: {err}");
                        }
                    }
                    let token = collection.security_tokens.remove(0);
                    let expiry = chrono::DateTime::parse_from_rfc3339(&token.lifetime.expires)?;
                    let token: Token = token.into();
                    let Token::Compact(user_token) = token else {
                        return Ok(vec![]);
                    };
                    let payload = MSATokenResponse {
                        gdk_session_cache_version: 1,
                        token: user_token,
                        puid: user.as_ref().map(|u| u.puid.clone()),
                        user_name: user.as_ref().map(|u| u.username.clone()),
                        expiry: expiry.timestamp(),
                        device_expiry: ms_device_rps_token.as_ref().map(|(_, r)| *r).unwrap_or(0),
                        device_rps: ms_device_rps_token
                            .map(|(t, _)| t)
                            .unwrap_or_else(String::new),
                    };
                    let payload = quick_xml::se::to_string(&payload)?;
                    Ok(payload.as_bytes().to_vec())
                }
                _ => Err("MSA authentication did not issue tokens".into()),
            }
        }
        XodusMessageType::XboxTokenRequest => {
            let req: XboxTokenRequest = quick_xml::de::from_reader(buffer.as_slice())?;
            let client_id = req.resolved_client_id()?.to_owned();
            let uri = reqwest::Url::parse(&req.relying_party)?;
            if !matches!(uri.scheme(), "https" | "wss")
                || uri.host_str().is_none()
                || !uri.username().is_empty()
                || uri.password().is_some()
                || uri.fragment().is_some()
            {
                tracing::warn!(
                    scheme = uri.scheme(),
                    host = uri.host_str().unwrap_or(""),
                    has_credentials = !uri.username().is_empty() || uri.password().is_some(),
                    has_fragment = uri.fragment().is_some(),
                    "Unsupported Xbox resource URL"
                );
                return Err("Unsupported Xbox relying party".into());
            }
            let user = context.tokens().get_user()?;
            // Do not reuse a session after local sign-out removed the credentials.
            context.tokens().get_user_sts_token()?;
            let slot = xodus::auth::broker_session_slot(&user.puid, &client_id, req.title_id);
            let mut cached = slot.lock().await;
            if !cached.as_ref().is_some_and(|session| session.is_valid()) {
                let (auth, sisu, device) = xodus::auth::do_sisu(
                    &context.client,
                    context.tokens(),
                    &client_id,
                    req.title_id,
                )
                .await
                .map_err(|_| "Xbox session authentication failed")?;
                let endpoints =
                    xodus::api::xbox::title::get_title_management(&context.client).await?;
                *cached = Some(xodus::auth::BrokerSession {
                    auth,
                    sisu,
                    device,
                    endpoints,
                    title_endpoints_loaded: false,
                    audiences: Default::default(),
                    created: std::time::Instant::now(),
                });
            }
            let session = cached.as_mut().ok_or("Xbox session unavailable")?;
            let xodus::auth::BrokerSession {
                auth,
                sisu,
                device,
                endpoints,
                title_endpoints_loaded,
                audiences,
                ..
            } = session;
            if !*title_endpoints_loaded
                && xodus::api::xbox::title::get_endpoint(&req.relying_party, endpoints).is_none()
            {
                let title_url = format!(
                    "https://title.mgt.xboxlive.com/titles/{}/endpoints",
                    req.title_id
                );
                let title_claims = sisu
                    .authorization_token
                    .display_claims
                    .as_ref()
                    .and_then(|c| c.xui.first())
                    .ok_or("No Xbox session claims")?;
                let title_uhs = title_claims.get("uhs").ok_or("No Xbox session user hash")?;
                let title_auth = format!("XBL3.0 x={title_uhs};{}", sisu.authorization_token.token);
                let signature = xodus::auth::sign_broker_request(
                    auth,
                    &title_url,
                    "GET",
                    "x-xbl-contract-version: 1\r\n",
                    &title_auth,
                )
                .await?;
                let title_endpoints: xodus::models::xbox::TitleMgtResponse = context
                    .client
                    .get(title_url)
                    .header("Authorization", title_auth)
                    .header("Signature", signature)
                    .header("x-xbl-contract-version", "1")
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                endpoints.end_points.extend(title_endpoints.end_points);
                *title_endpoints_loaded = true;
            }
            if xodus::api::xbox::title::get_endpoint(&req.relying_party, endpoints).is_none() {
                tracing::warn!(
                    host = uri.host_str().unwrap_or(""),
                    "Unrecognized Xbox resource host"
                );
            }
            let endpoint = xodus::api::xbox::title::get_endpoint(&req.relying_party, endpoints)
                .ok_or("Unknown Xbox resource endpoint")?;
            let relying_party = endpoint
                .relying_party
                .as_deref()
                .ok_or("Endpoint has no token audience")?;
            let token = if relying_party == "http://xboxlive.com" {
                sisu.authorization_token.clone()
            } else {
                let margin = chrono::Utc::now() + chrono::Duration::seconds(60);
                if let Some(token) = audiences
                    .get(relying_party)
                    .filter(|token| token.not_after > margin)
                {
                    token.clone()
                } else {
                    let token = auth
                        .get_xsts_token(
                            Some(device),
                            Some(&sisu.title_token),
                            Some(&sisu.user_token),
                            relying_party,
                        )
                        .await
                        .map_err(|_| "Xbox relying-party authorization failed")?;
                    audiences.insert(relying_party.to_owned(), token.clone());
                    token
                }
            };
            let claims = sisu
                .authorization_token
                .display_claims
                .as_ref()
                .and_then(|c| c.xui.first())
                .ok_or("Xbox session has no user claims")?;
            let uhs = token
                .display_claims
                .as_ref()
                .and_then(|c| c.xui.first())
                .and_then(|c| c.get("uhs"))
                .ok_or("Xbox response has no user hash")?;
            let xuid = claims.get("xid").ok_or("Xbox response has no user ID")?;
            let authorization = format!("XBL3.0 x={uhs};{}", token.token);
            let signature =
                if req.http_method.is_empty() || endpoint.signature_policy_index.is_none() {
                    String::new()
                } else {
                    xodus::auth::sign_broker_request(
                        auth,
                        &req.relying_party,
                        &req.http_method,
                        &req.request_headers,
                        &authorization,
                    )
                    .await?
                };
            let payload = XboxTokenResponse {
                token: authorization,
                signature,
                sandbox: auth.sandbox_id(),
                expiry: token.not_after.timestamp(),
                puid: user.puid,
                user_name: user.username,
                xuid: xuid.clone(),
                gamertag: claims.get("gtg").cloned().unwrap_or_default(),
                user_hash: uhs.clone(),
                age_group: claims
                    .get("agg")
                    .cloned()
                    .ok_or("Xbox response has no age group")?,
                modern_gamertag: claims.get("mgt").cloned().unwrap_or_default(),
                modern_gamertag_suffix: claims.get("mgs").cloned().unwrap_or_default(),
                unique_modern_gamertag: claims.get("umg").cloned().unwrap_or_default(),
                privileges: claims.get("prv").cloned().unwrap_or_default(),
                enforcement: claims.get("enf").cloned().unwrap_or_default(),
                restrictions: claims.get("usr").cloned().unwrap_or_default(),
                title_restrictions: claims.get("utr").cloned().unwrap_or_default(),
            };
            Ok(quick_xml::se::to_string(&payload)?.into_bytes())
        }
        XodusMessageType::StoreRequest => {
            let req: xodus::licensing::store::StoreRequest =
                quick_xml::de::from_reader(buffer.as_slice())?;
            let reply = xodus::licensing::store::handle(context.tokens(), req).await;
            let payload = quick_xml::se::to_string(&reply)?;
            if payload.len() > u16::MAX as usize {
                return Err("Store response exceeds protocol limit".into());
            }
            Ok(payload.into_bytes())
        }
        _ => Err("Unimplemented".into()),
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use std::sync::Arc;
    use xodus::licensing::ownership::{OwnershipResponse, OwnershipStatus};
    use xodus::models::secrets::LegacyToken;
    use xodus::tokens::TokenManager;

    fn context() -> SimpleContext {
        SimpleContext::new(
            LegacyToken {
                key_name: None,
                token: String::new(),
                binary_secret: None,
                tpm_key: None,
                lifetime: soap::Timestamp {
                    id: None,
                    created: String::new(),
                    expires: String::new(),
                },
            },
            Arc::new(TokenManager::with_memory()),
        )
    }

    #[tokio::test]
    async fn no_account_and_malformed_requests_return_typed_responses() {
        for (payload, expected) in [
            (
                "<OwnershipRequest><productId>9NBLGGH2JHXJ</productId></OwnershipRequest>",
                OwnershipStatus::NoAccount,
            ),
            (
                "<OwnershipRequest><productId>bad</productId></OwnershipRequest>",
                OwnershipStatus::InvalidRequest,
            ),
            ("<broken", OwnershipStatus::InvalidRequest),
            ("", OwnershipStatus::InvalidRequest),
        ] {
            let bytes = parse_message(
                &mut context(),
                XodusMessageType::OwnershipRequest,
                payload.as_bytes().to_vec(),
            )
            .await
            .unwrap();
            let response: OwnershipResponse = quick_xml::de::from_reader(bytes.as_slice()).unwrap();
            assert_eq!(response.status, expected);
        }
    }

    #[tokio::test]
    async fn ping_protocol_is_unchanged() {
        assert_eq!(
            parse_message(&mut context(), XodusMessageType::Ping, b"hello".to_vec())
                .await
                .unwrap(),
            b"hello"
        );
    }
}
