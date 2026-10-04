use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::models::secrets::{Device, SavedAccount, Token, TokenStore, User};
use crate::models::xbox::XstsResponse;
use crate::tokens::backend::{KeychainBackend, MemoryBackend};
use crate::tokens::store::{ExpiringTokenBackend, TokenBackend, TokenStoreError};

pub(crate) mod keys {
    pub const DEV_LICENSE: &str = "dev_license";
    pub const DEVICE_TOKENS: &str = "device-tokens";
    pub const USER_TOKENS: &str = "user-tokens";
    pub const USER_INFO: &str = "user-DA";
    pub const SAVED_ACCOUNTS: &str = "saved-accounts-v1";
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn session_accounts_do_not_change_profile_or_other_sessions() {
        let profile = TokenManager::with_memory();
        sign_in(&profile, "first");
        sign_in(&profile, "second");
        let accounts = profile.accounts().unwrap();
        let first = accounts.iter().find(|a| a.username == "first").unwrap();
        let second = accounts.iter().find(|a| a.username == "second").unwrap();
        let session_a = profile.session_for_account(&first.id).unwrap();
        let session_b = profile.session_for_account(&second.id).unwrap();
        session_a
            .save_user_token(
                PASSPORT_STS.into(),
                Token::Compact("refreshed-first".into()),
            )
            .unwrap();
        assert_eq!(profile.get_user().unwrap().puid, "second");
        assert_eq!(session_a.get_user().unwrap().puid, "first");
        assert_eq!(session_b.get_user().unwrap().puid, "second");
        assert!(
            matches!(session_b.get_user_sts_token().unwrap(), Token::Compact(t) if t == "test-second")
        );
        assert!(
            matches!(profile.session_for_account(&first.id).unwrap().get_user_sts_token().unwrap(), Token::Compact(t) if t == "test-first")
        );
        let content = uuid::Uuid::from_u128(42);
        profile.save_installed_key(content, &[5; 32]).unwrap();
        assert_eq!(session_a.installed_key(content).unwrap(), [5; 32]);
        assert!(profile.session_for_account("missing").is_err());
        assert_eq!(profile.get_user().unwrap().puid, "second");
        assert_eq!(profile.accounts().unwrap().len(), 2);
    }

    fn sign_in(tokens: &TokenManager, id: &str) {
        tokens
            .save_user(&User {
                puid: id.into(),
                username: id.into(),
            })
            .unwrap();
        tokens
            .save_user_token(PASSPORT_STS.into(), Token::Compact(format!("test-{id}")))
            .unwrap();
        tokens.remember_current_account().unwrap();
    }

    #[test]
    fn accounts_stay_in_profile_and_logout_removes_all_of_them() {
        let profile = TokenManager::with_memory();
        let other = TokenManager::with_memory();
        sign_in(&profile, "first");
        sign_in(&profile, "second");
        sign_in(&profile, "first");
        let accounts = profile.ownership_accounts().unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].user.puid, "first");
        assert_eq!(accounts[1].user.puid, "second");
        assert!(other.ownership_accounts().unwrap().is_empty());
        profile.remove_persistent().unwrap();
        assert!(profile.ownership_accounts().unwrap().is_empty());
    }

    #[test]
    fn changing_users_cannot_relabel_previous_credentials() {
        let profile = TokenManager::with_memory();
        sign_in(&profile, "first");
        profile
            .save_user(&User {
                puid: "second".into(),
                username: "second".into(),
            })
            .unwrap();
        assert!(profile.get_user_sts_token().is_err());
    }

    #[test]
    fn installed_keys_are_profile_and_content_scoped_and_survive_logout() {
        let profile = TokenManager::with_memory();
        let other = TokenManager::with_memory();
        let content = uuid::Uuid::from_u128(1);
        profile.save_installed_key(content, &[7; 32]).unwrap();
        profile.remove_persistent().unwrap();
        assert_eq!(profile.installed_key(content).unwrap(), [7; 32]);
        assert!(other.installed_key(content).is_err());
        assert!(profile.installed_key(uuid::Uuid::from_u128(2)).is_err());
    }

    #[test]
    fn selecting_and_removing_accounts_preserves_other_credentials_and_installed_keys() {
        let profile = TokenManager::with_memory();
        sign_in(&profile, "first");
        sign_in(&profile, "second");
        let first = profile
            .accounts()
            .unwrap()
            .into_iter()
            .find(|a| a.username == "first")
            .unwrap();
        let second = profile
            .accounts()
            .unwrap()
            .into_iter()
            .find(|a| a.username == "second")
            .unwrap();
        let content = uuid::Uuid::from_u128(1);
        profile.save_installed_key(content, &[9; 32]).unwrap();
        profile.select_account(&first.id).unwrap();
        assert_eq!(profile.get_user().unwrap().puid, "first");
        assert!(
            matches!(profile.get_user_sts_token().unwrap(), Token::Compact(token) if token == "test-first")
        );
        assert_eq!(
            profile
                .accounts()
                .unwrap()
                .iter()
                .filter(|a| a.active)
                .count(),
            1
        );
        assert!(profile.select_account("unknown").is_err());
        assert!(profile.remove_account("unknown").is_err());
        assert_eq!(profile.accounts().unwrap().len(), 2);
        profile.remove_account(&first.id).unwrap();
        assert_eq!(profile.get_user().unwrap().puid, "second");
        assert!(
            matches!(profile.get_user_sts_token().unwrap(), Token::Compact(token) if token == "test-second")
        );
        profile.remove_account(&second.id).unwrap();
        assert!(profile.accounts().unwrap().is_empty());
        assert!(profile.get_user_sts_token().is_err());
        assert_eq!(profile.installed_key(content).unwrap(), [9; 32]);
    }

    #[test]
    fn public_summaries_never_include_tokens_or_raw_user_ids() {
        let profile = TokenManager::with_memory();
        sign_in(&profile, "private-user-id");
        profile
            .save_user(&User {
                puid: "private-user-id".into(),
                username: "Player".into(),
            })
            .unwrap();
        let json = serde_json::to_string(&profile.accounts().unwrap()).unwrap();
        assert!(json.contains("Player"));
        assert!(!json.contains("private-user-id"));
        assert!(!json.contains("test-"));
        assert!(!json.contains("sts"));
    }

    #[test]
    fn inactive_removal_does_not_switch_current_user() {
        let profile = TokenManager::with_memory();
        sign_in(&profile, "first");
        sign_in(&profile, "second");
        let first = profile
            .accounts()
            .unwrap()
            .into_iter()
            .find(|a| !a.active)
            .unwrap();
        profile.remove_account(&first.id).unwrap();
        assert_eq!(profile.get_user().unwrap().puid, "second");
        assert_eq!(profile.ownership_accounts().unwrap().len(), 1);
    }

    #[test]
    fn account_switch_clears_ephemeral_cache() {
        let memory = Arc::new(MemoryBackend::default());
        let profile = TokenManager::new(Arc::new(MemoryBackend::default()), memory.clone());
        sign_in(&profile, "first");
        sign_in(&profile, "second");
        memory.set("cached-xsts", b"private").unwrap();
        profile.cache_gdk_sessions(b"paired-key-and-token").unwrap();
        let first = profile
            .accounts()
            .unwrap()
            .into_iter()
            .find(|a| !a.active)
            .unwrap();
        profile.select_account(&first.id).unwrap();
        assert!(memory.get("cached-xsts").unwrap().is_none());
        assert!(profile.get_cached_gdk_sessions().is_none());
        profile.cache_gdk_sessions(b"new-session").unwrap();
        profile.remove_persistent().unwrap();
        assert!(profile.get_cached_gdk_sessions().is_none());
    }
}

pub const PASSPORT_STS: &str = "http://Passport.NET/STS";

/// Safe to return to frontends. Never serialize SavedAccount outside secure storage.
#[derive(serde::Serialize)]
pub struct AccountSummary {
    pub id: String,
    pub username: String,
    pub active: bool,
}

fn account_id(user: &User) -> String {
    Sha256::digest(user.puid.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Semantic facade over the two storage tiers: a persistent, keychain-backed tier
/// for STS/device/user credentials, and an ephemeral tier for short-lived
/// per-relying-party XSTS tokens. Centralizes the read-merge-write pattern that was
/// previously duplicated across `xodus-cli` and `xodus-service`.
#[derive(Clone)]
pub struct TokenManager {
    persistent: Arc<dyn TokenBackend>,
    ephemeral: Arc<dyn ExpiringTokenBackend>,
}

impl TokenManager {
    pub fn new(
        persistent: Arc<dyn TokenBackend>,
        ephemeral: Arc<dyn ExpiringTokenBackend>,
    ) -> Self {
        Self {
            persistent,
            ephemeral,
        }
    }

    /// Keychain for persistent storage, in-memory for ephemeral - the default
    /// wiring for both `xodus-cli` and `xodus-service` today.
    pub fn with_keychain_and_memory() -> Self {
        Self::new(
            Arc::new(KeychainBackend),
            Arc::new(MemoryBackend::default()),
        )
    }

    /// Keychain for persistent storage, in-memory for ephemeral - the default
    /// wiring for both `xodus-cli` and `xodus-service` today.
    pub fn with_memory() -> Self {
        Self::new(
            Arc::new(MemoryBackend::default()),
            Arc::new(MemoryBackend::default()),
        )
    }

    /// Pin one saved identity without changing the profile's current account.
    /// Refreshed user tokens stay process-local to this service session.
    pub fn session_for_account(&self, id: &str) -> Result<Self, TokenStoreError> {
        let accounts = self.ownership_accounts()?;
        let account = accounts
            .iter()
            .find(|a| account_id(&a.user) == id)
            .ok_or(TokenStoreError::NotFound)?;
        let session = Self::new(
            Arc::new(crate::tokens::backend::session::SessionBackend {
                profile: self.persistent.clone(),
                identity: MemoryBackend::default(),
            }),
            Arc::new(MemoryBackend::default()),
        );
        session
            .persistent
            .set(keys::SAVED_ACCOUNTS, &serde_json::to_vec(&[account])?)?;
        session.activate_account(Some(account))?;
        Ok(session)
    }

    pub fn remove_persistent(&self) -> Result<(), TokenStoreError> {
        self.ephemeral.clear()?;
        self.persistent.remove(keys::SAVED_ACCOUNTS)?;
        self.persistent.remove(keys::DEVICE_TOKENS)?;
        self.persistent.remove(keys::USER_TOKENS)?;
        self.persistent.remove(keys::USER_INFO)
    }

    // ---- Device identity / license -----------------------------------------

    pub fn get_device_license(&self) -> Result<Device, TokenStoreError> {
        let bytes = self
            .persistent
            .get(keys::DEV_LICENSE)?
            .ok_or(TokenStoreError::NotFound)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save_device_license(&self, device: &Device) -> Result<(), TokenStoreError> {
        self.persistent
            .set(keys::DEV_LICENSE, &serde_json::to_vec(device)?)
    }

    pub fn remove_device_license(&self) -> Result<(), TokenStoreError> {
        self.persistent.remove(keys::DEV_LICENSE)
    }

    // ---- Device STS tokens (keyed by SOAP "applies_to" address) -----------

    pub fn get_device_token_for(&self, address: &str) -> Result<Option<Token>, TokenStoreError> {
        Self::read_token_store(&*self.persistent, keys::DEVICE_TOKENS, address)
    }

    pub fn save_device_token(&self, address: String, token: Token) -> Result<(), TokenStoreError> {
        Self::write_token_store(&*self.persistent, keys::DEVICE_TOKENS, address, token)
    }

    pub fn get_device_sts_token(&self) -> Result<Token, TokenStoreError> {
        self.get_device_token_for(PASSPORT_STS)?
            .ok_or(TokenStoreError::NotFound)
    }

    // ---- User STS tokens (keyed by SOAP "applies_to" address) --------------

    pub fn get_user_token_for(&self, address: &str) -> Result<Option<Token>, TokenStoreError> {
        Self::read_token_store(&*self.persistent, keys::USER_TOKENS, address)
    }

    pub fn save_user_token(&self, address: String, token: Token) -> Result<(), TokenStoreError> {
        Self::write_token_store(&*self.persistent, keys::USER_TOKENS, address, token)
    }

    pub fn get_user_sts_token(&self) -> Result<Token, TokenStoreError> {
        self.get_user_token_for(PASSPORT_STS)?
            .ok_or(TokenStoreError::NotFound)
    }

    // ---- User info -----------------------------------------------------------

    pub fn get_user(&self) -> Result<User, TokenStoreError> {
        let bytes = self
            .persistent
            .get(keys::USER_INFO)?
            .ok_or(TokenStoreError::NotFound)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save_user(&self, user: &User) -> Result<(), TokenStoreError> {
        self.ephemeral.clear()?;
        if self
            .get_user()
            .is_ok_and(|previous| previous.puid != user.puid)
        {
            // Never associate the previous account's token with a new login.
            if self.get_user_sts_token().is_ok() {
                self.remember_current_account()?;
            }
            self.persistent.remove(keys::USER_TOKENS)?;
        }
        self.persistent
            .set(keys::USER_INFO, &serde_json::to_vec(user)?)
    }

    /// Called only after a successful login has persisted both user and STS.
    pub fn remember_current_account(&self) -> Result<(), TokenStoreError> {
        let mut accounts = self.ownership_accounts()?;
        let current = SavedAccount {
            user: self.get_user()?,
            sts: self.get_user_sts_token()?,
        };
        accounts.retain(|a| a.user.puid != current.user.puid);
        accounts.insert(0, current);
        self.persistent
            .set(keys::SAVED_ACCOUNTS, &serde_json::to_vec(&accounts)?)
    }

    pub fn accounts(&self) -> Result<Vec<AccountSummary>, TokenStoreError> {
        let active = match self.get_user() {
            Ok(user) => Some(user.puid),
            Err(TokenStoreError::NotFound) => None,
            Err(error) => return Err(error),
        };
        Ok(self
            .ownership_accounts()?
            .iter()
            .map(|account| AccountSummary {
                id: account_id(&account.user),
                username: account.user.username.clone(),
                active: active.as_ref() == Some(&account.user.puid),
            })
            .collect())
    }

    pub fn select_account(&self, id: &str) -> Result<(), TokenStoreError> {
        let accounts = self.ownership_accounts()?;
        let account = accounts
            .iter()
            .find(|a| account_id(&a.user) == id)
            .ok_or(TokenStoreError::NotFound)?;
        // Preserve every account before replacing the active identity. On failure,
        // credentials can never be associated with the wrong user.
        self.persistent
            .set(keys::SAVED_ACCOUNTS, &serde_json::to_vec(&accounts)?)?;
        self.activate_account(Some(account))
    }

    pub fn remove_account(&self, id: &str) -> Result<(), TokenStoreError> {
        let mut accounts = self.ownership_accounts()?;
        let index = accounts
            .iter()
            .position(|a| account_id(&a.user) == id)
            .ok_or(TokenStoreError::NotFound)?;
        let removed = accounts.remove(index);
        if self
            .get_user()
            .is_ok_and(|user| user.puid == removed.user.puid)
        {
            self.activate_account(accounts.first())?;
        }
        self.persistent
            .set(keys::SAVED_ACCOUNTS, &serde_json::to_vec(&accounts)?)
    }

    fn activate_account(&self, account: Option<&SavedAccount>) -> Result<(), TokenStoreError> {
        self.ephemeral.clear()?;
        self.persistent.remove(keys::USER_TOKENS)?;
        self.persistent.remove(keys::USER_INFO)?;
        if let Some(account) = account {
            self.persistent
                .set(keys::USER_INFO, &serde_json::to_vec(&account.user)?)?;
            self.save_user_token(PASSPORT_STS.into(), account.sts.clone())?;
        }
        Ok(())
    }

    /// Installed content is deliberately retained across account logout. These
    /// keys are scoped to this profile, never placed in the game folder or IPC.
    pub fn save_installed_key(
        &self,
        content_id: uuid::Uuid,
        key: &[u8; 32],
    ) -> Result<(), TokenStoreError> {
        self.persistent
            .set(&format!("installed-content-v1:{content_id}"), key)
    }

    pub fn installed_key(&self, content_id: uuid::Uuid) -> Result<[u8; 32], TokenStoreError> {
        let bytes = self
            .persistent
            .get(&format!("installed-content-v1:{content_id}"))?
            .ok_or(TokenStoreError::NotFound)?;
        bytes
            .try_into()
            .map_err(|_| std::io::Error::other("Invalid installed content key").into())
    }

    /// Legacy single-account profiles work without copying secrets from another
    /// Xodus profile or switching the active Xbox account.
    pub fn ownership_accounts(&self) -> Result<Vec<SavedAccount>, TokenStoreError> {
        let mut accounts: Vec<SavedAccount> = match self.persistent.get(keys::SAVED_ACCOUNTS)? {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => Vec::new(),
        };
        match self.get_user() {
            Ok(user) => {
                let sts = self.get_user_sts_token()?;
                accounts.retain(|a| a.user.puid != user.puid);
                accounts.insert(0, SavedAccount { user, sts });
            }
            Err(TokenStoreError::NotFound) => {}
            Err(error) => return Err(error),
        }
        Ok(accounts)
    }

    /// Wine's proof-key/token pairs are volatile and share the logout/account
    /// invalidation rules of the other ephemeral authentication state.
    pub fn get_cached_gdk_sessions(&self) -> Option<Vec<u8>> {
        self.ephemeral.get("winegdk-bootstrap-v1").ok().flatten()
    }

    pub fn cache_gdk_sessions(&self, data: &[u8]) -> Result<(), TokenStoreError> {
        if data.len() > 1_000_000 {
            return Ok(());
        }
        self.ephemeral.set_with_expiry(
            "winegdk-bootstrap-v1",
            data,
            Instant::now() + std::time::Duration::from_secs(3600),
        )
    }

    // ---- Ephemeral XSTS-by-relying-party cache --------------------------------

    pub fn get_cached_xsts(&self, relying_party: &str) -> Option<XstsResponse> {
        let bytes = self.ephemeral.get(relying_party).ok()??;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn cache_xsts(&self, relying_party: &str, token: &XstsResponse) {
        self.cache_xsts_response(relying_party, token);
    }

    fn cache_xsts_response(&self, key: &str, token: &XstsResponse) {
        let Ok(bytes) = serde_json::to_vec(token) else {
            return;
        };
        let remaining = (token.not_after - chrono::Utc::now())
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);
        let _ = self
            .ephemeral
            .set_with_expiry(key, &bytes, Instant::now() + remaining);
    }

    // ---- shared TokenStore read/modify/write helper ---------------------------

    fn read_token_store(
        backend: &dyn TokenBackend,
        key: &str,
        address: &str,
    ) -> Result<Option<Token>, TokenStoreError> {
        let Some(bytes) = backend.get(key)? else {
            return Ok(None);
        };
        let store: TokenStore = serde_json::from_slice(&bytes)?;
        Ok(store.tokens.get(address).cloned())
    }

    fn write_token_store(
        backend: &dyn TokenBackend,
        key: &str,
        address: String,
        token: Token,
    ) -> Result<(), TokenStoreError> {
        let mut tokens: HashMap<String, Token> = match backend.get(key)? {
            Some(bytes) if !bytes.is_empty() => {
                serde_json::from_slice::<TokenStore>(&bytes)?.tokens
            }
            _ => HashMap::new(),
        };
        tokens.insert(address, token);
        backend.set(key, &serde_json::to_vec(&TokenStore { tokens })?)
    }
}
