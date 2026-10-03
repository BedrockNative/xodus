use std::sync::Arc;

use super::MemoryBackend;
use crate::tokens::manager::keys;
use crate::tokens::store::{TokenBackend, TokenStoreError};

/// Identity writes stay process-local; device identity and installed content
/// remain in the original private profile. No credentials are exported to disk.
pub(crate) struct SessionBackend {
    pub profile: Arc<dyn TokenBackend>,
    pub identity: MemoryBackend,
}

impl SessionBackend {
    fn backend(&self, key: &str) -> &dyn TokenBackend {
        match key {
            keys::USER_TOKENS | keys::USER_INFO | keys::SAVED_ACCOUNTS => &self.identity,
            _ => &*self.profile,
        }
    }
}

impl TokenBackend for SessionBackend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, TokenStoreError> {
        self.backend(key).get(key)
    }
    fn set(&self, key: &str, value: &[u8]) -> Result<(), TokenStoreError> {
        self.backend(key).set(key, value)
    }
    fn remove(&self, key: &str) -> Result<(), TokenStoreError> {
        self.backend(key).remove(key)
    }
}
