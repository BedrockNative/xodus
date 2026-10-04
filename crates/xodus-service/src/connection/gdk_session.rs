//! Volatile cache of WineGDK's authenticated bootstrap state. Never persisted or logged.
//! The opaque payload keeps proof keys paired with their tokens. MSA credentials
//! are still obtained afresh; this cache cannot select or sign in an account.
use crate::simple_context::SimpleContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

const MAX_DATA: usize = 60_000;
const MAX_ENTRIES: usize = 16;
const MARGIN: i64 = 60;
const MAX_AGE: i64 = 3600; // Refresh profile/endpoint metadata at least hourly.

#[derive(Deserialize, Serialize)]
#[serde(
    rename = "GdkSessionRequest",
    rename_all = "PascalCase",
    deny_unknown_fields
)]
pub struct Request {
    operation: String,
    puid: String,
    client_id: String,
    title_id: u32,
    full_trust: bool,
    #[serde(default)]
    expiry: i64,
    #[serde(default)]
    data: String,
}
#[derive(Serialize, Default)]
#[serde(rename = "GdkSessionResponse", rename_all = "PascalCase")]
pub struct Response {
    status: &'static str,
    expiry: i64,
    data: String,
}
#[derive(Hash, PartialEq, Eq, Serialize, Deserialize)]
struct Key(String, String, u32, bool);
#[derive(Serialize, Deserialize)]
struct Entry {
    expiry: i64,
    data: String,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<Key, Entry>,
}
impl Cache {
    fn exchange(&mut self, req: Request, account: Option<&str>, now: i64) -> Response {
        let miss = || Response {
            status: "miss",
            ..Default::default()
        };
        if req.puid.is_empty()
            || req.puid.len() > 256
            || req.client_id.is_empty()
            || req.client_id.len() > 256
            || req.title_id == 0
        {
            return miss();
        }
        if account != Some(req.puid.as_str()) {
            self.entries.retain(|key, _| key.0 != req.puid);
            return miss();
        }
        self.entries
            .retain(|_, entry| entry.expiry > now.saturating_add(MARGIN));
        let key = Key(req.puid, req.client_id, req.title_id, req.full_trust);
        match req.operation.as_str() {
            "get" => match self.entries.get(&key) {
                Some(entry) => Response {
                    status: "hit",
                    expiry: entry.expiry,
                    data: entry.data.clone(),
                },
                None => miss(),
            },
            "put" => {
                if req.expiry <= now.saturating_add(MARGIN)
                    || req.data.is_empty()
                    || req.data.len() > MAX_DATA
                    || req.data.len() % 4 != 0
                    || !req
                        .data
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(&c))
                {
                    return miss();
                }
                let expiry = req.expiry.min(now.saturating_add(MAX_AGE));
                if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
                    self.entries.clear();
                }
                self.entries.insert(
                    key,
                    Entry {
                        expiry,
                        data: req.data,
                    },
                );
                Response {
                    status: "stored",
                    expiry,
                    ..Default::default()
                }
            }
            "invalidate" => {
                self.entries.remove(&key);
                Response {
                    status: "invalidated",
                    ..Default::default()
                }
            }
            _ => miss(),
        }
    }
}

pub fn handle(
    context: &SimpleContext,
    request: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    static CACHE_LOCK: Mutex<()> = Mutex::new(());
    let _guard = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let request: Request =
        quick_xml::de::from_reader(request).map_err(|_| "Invalid GDK session request")?;
    let user = context.tokens().get_user().ok();
    let authenticated = context.tokens().get_user_sts_token().is_ok();
    let account = user
        .as_ref()
        .filter(|_| authenticated)
        .map(|u| u.puid.as_str());
    let entries: Vec<(Key, Entry)> = context
        .tokens()
        .get_cached_gdk_sessions()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut cache = Cache {
        entries: entries.into_iter().collect(),
    };
    let response = cache.exchange(request, account, chrono::Utc::now().timestamp());
    let entries: Vec<_> = cache.entries.into_iter().collect();
    context
        .tokens()
        .cache_gdk_sessions(&serde_json::to_vec(&entries)?)?;
    Ok(quick_xml::se::to_string(&response)?.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(operation: &str) -> Request {
        Request {
            operation: operation.into(),
            puid: "alice".into(),
            client_id: "app-a".into(),
            title_id: 42,
            full_trust: false,
            expiry: 2000,
            data: "c2VjcmV0".into(),
        }
    }
    #[test]
    fn survives_connections_but_not_expiry() {
        let mut cache = Cache::default();
        assert_eq!(
            cache.exchange(req("put"), Some("alice"), 1000).status,
            "stored"
        );
        assert_eq!(
            cache.exchange(req("get"), Some("alice"), 1200).data,
            "c2VjcmV0"
        );
        assert_eq!(
            cache.exchange(req("get"), Some("alice"), 1940).status,
            "miss"
        );
    }
    #[test]
    fn isolates_account_application_title_and_trust() {
        let mut cache = Cache::default();
        cache.exchange(req("put"), Some("alice"), 1000);
        let mut request = req("get");
        request.puid = "bob".into();
        assert_eq!(cache.exchange(request, Some("bob"), 1000).status, "miss");
        let mut request = req("get");
        request.client_id = "app-b".into();
        assert_eq!(cache.exchange(request, Some("alice"), 1000).status, "miss");
        let mut request = req("get");
        request.title_id += 1;
        assert_eq!(cache.exchange(request, Some("alice"), 1000).status, "miss");
        let mut request = req("get");
        request.full_trust = true;
        assert_eq!(cache.exchange(request, Some("alice"), 1000).status, "miss");
        assert_eq!(
            cache.exchange(req("get"), Some("alice"), 1000).status,
            "hit"
        );
    }
    #[test]
    fn signout_account_switch_and_force_refresh_discard_state() {
        for account in [None, Some("bob")] {
            let mut cache = Cache::default();
            cache.exchange(req("put"), Some("alice"), 1000);
            assert_eq!(cache.exchange(req("get"), account, 1000).status, "miss");
            assert_eq!(
                cache.exchange(req("get"), Some("alice"), 1000).status,
                "miss"
            );
        }
        let mut cache = Cache::default();
        cache.exchange(req("put"), Some("alice"), 1000);
        assert_eq!(
            cache
                .exchange(req("invalidate"), Some("alice"), 1000)
                .status,
            "invalidated"
        );
        assert_eq!(
            cache.exchange(req("get"), Some("alice"), 1000).status,
            "miss"
        );
    }
    #[test]
    fn rejects_expired_and_oversized_state_and_bounds_metadata_age() {
        let mut cache = Cache::default();
        let mut request = req("put");
        request.expiry = 1060;
        assert_eq!(cache.exchange(request, Some("alice"), 1000).status, "miss");
        let mut request = req("put");
        request.data = "A".repeat(MAX_DATA + 4);
        assert_eq!(cache.exchange(request, Some("alice"), 1000).status, "miss");
        let mut request = req("put");
        request.expiry = i64::MAX;
        assert_eq!(cache.exchange(request, Some("alice"), 1000).expiry, 4600);
    }
    #[test]
    fn wire_round_trip_survives_connections_and_obeys_logout() {
        use std::sync::Arc;
        use xodus::models::secrets::{LegacyToken, Token, User};
        use xodus::models::soap::Timestamp;
        use xodus::tokens::{TokenManager, manager::PASSPORT_STS};
        let tokens = Arc::new(TokenManager::with_memory());
        let make_context = || {
            SimpleContext::new(
                LegacyToken {
                    key_name: None,
                    token: String::new(),
                    binary_secret: None,
                    tpm_key: None,
                    lifetime: Timestamp {
                        id: None,
                        created: String::new(),
                        expires: String::new(),
                    },
                },
                tokens.clone(),
            )
        };
        let sign_in = || {
            tokens
                .save_user(&User {
                    puid: "alice".into(),
                    username: "fixture".into(),
                })
                .unwrap();
            tokens
                .save_user_token(PASSPORT_STS.into(), Token::Compact("fixture-sts".into()))
                .unwrap();
        };
        let exchange = |context: &SimpleContext, operation: &str| {
            let mut request = req(operation);
            request.expiry = chrono::Utc::now().timestamp() + 1200;
            let xml = quick_xml::se::to_string(&request).unwrap();
            String::from_utf8(handle(context, xml.as_bytes()).unwrap()).unwrap()
        };
        sign_in();
        assert!(exchange(&make_context(), "put").contains("<Status>stored</Status>"));
        let reply = exchange(&make_context(), "get");
        assert!(reply.contains("<Status>hit</Status>"));
        assert!(reply.contains("<Data>c2VjcmV0</Data>"));
        tokens.remove_persistent().unwrap();
        assert!(exchange(&make_context(), "get").contains("<Status>miss</Status>"));
        sign_in();
        assert!(exchange(&make_context(), "get").contains("<Status>miss</Status>"));
        assert_eq!(
            xodus::proto::xodus::XodusMessageType::GdkSessionRequest as u16,
            11
        );
        assert_eq!(
            xodus::proto::xodus::XodusMessageType::GdkSessionResponse as u16,
            12
        );
    }
}
