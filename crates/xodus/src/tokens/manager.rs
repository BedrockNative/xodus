use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::models::secrets::{Device, SavedAccount, Token, TokenStore, User};
use crate::models::xbox::XstsResponse;
use crate::tokens::backend::{KeychainBackend, MemoryBackend};
use crate::tokens::store::{ExpiringTokenBackend, TokenBackend, TokenStoreError};

mod keys {
    pub const DEV_LICENSE: &str = "dev_license";
    pub const DEVICE_TOKENS: &str = "device-tokens";
    pub const USER_TOKENS: &str = "user-tokens";
    pub const USER_INFO: &str = "user-DA";
    pub const SAVED_ACCOUNTS: &str = "saved-accounts-v1";
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

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
}

pub const PASSPORT_STS: &str = "http://Passport.NET/STS";

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

    pub fn remove_persistent(&self) -> Result<(), TokenStoreError> {
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
