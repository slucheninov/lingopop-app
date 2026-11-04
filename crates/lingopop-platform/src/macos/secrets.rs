//! Keychain-backed native macOS protector. Construction performs no Keychain access.
use super::keychain::LoginKeyStore;
use crate::secrets_aead::KeyStoreProtector;
use lingopop_core::secrets::{ProtectError, SecretProtector};
use std::sync::Arc;
#[derive(Default)]
pub struct MacProtector(KeyStoreProtector);
impl MacProtector {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Default for KeyStoreProtector {
    fn default() -> Self {
        Self::new(Arc::new(LoginKeyStore::default()))
    }
}
impl SecretProtector for MacProtector {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
        self.0.protect(plain)
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
        self.0.unprotect(data)
    }
}
