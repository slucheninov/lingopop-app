//! Portable authenticated container: LPS1 || nonce || AES-256-GCM ciphertext/tag.
use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use lingopop_core::secrets::ProtectError;
use zeroize::{Zeroize, Zeroizing};
const MAGIC: &[u8] = b"LPS1";

/// Key material is never formatted and is cleared when dropped.
#[derive(Clone)]
pub struct SecretKey(Zeroizing<[u8; 32]>);
impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}
impl SecretKey {
    pub fn generate() -> Result<Self, ProtectError> {
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(key.as_mut()).map_err(|_| ProtectError::Unavailable)?;
        Ok(Self(key))
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtectError> {
        if bytes.len() != 32 {
            return Err(ProtectError::Unavailable);
        }
        let mut key = Zeroizing::new([0; 32]);
        key.copy_from_slice(bytes);
        Ok(Self(key))
    }
    /// For the key-store boundary only. Never log this value.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }
}
pub fn seal(key: &SecretKey, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
    let mut nonce = [0; 12];
    getrandom::fill(&mut nonce).map_err(|_| ProtectError::Unavailable)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| ProtectError::Invalid)?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: MAGIC,
            },
        )
        .map_err(|_| ProtectError::Invalid)?;
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&nonce);
    out.extend(encrypted);
    nonce.zeroize();
    Ok(out)
}
pub fn open(key: &SecretKey, bytes: &[u8]) -> Result<Vec<u8>, ProtectError> {
    if bytes.len() < 32 || !bytes.starts_with(MAGIC) {
        return Err(ProtectError::Invalid);
    }
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| ProtectError::Invalid)?;
    cipher
        .decrypt(
            Nonce::from_slice(&bytes[4..16]),
            Payload {
                msg: &bytes[16..],
                aad: MAGIC,
            },
        )
        .map_err(|_| ProtectError::Invalid)
}

/// Atomic insertion MUST return the existing durable key on a duplicate; never update it.
pub trait KeychainKeyStore: Send + Sync {
    fn load(&self) -> Result<Option<SecretKey>, ProtectError>;
    fn insert_if_absent(&self, key: SecretKey) -> Result<SecretKey, ProtectError>;
    fn delete(&self) -> Result<(), ProtectError>;
}
/// Portable protector logic; native Keychain operations are injected.
pub struct KeyStoreProtector {
    keys: std::sync::Arc<dyn KeychainKeyStore>,
}
impl KeyStoreProtector {
    pub fn new(keys: std::sync::Arc<dyn KeychainKeyStore>) -> Self {
        Self { keys }
    }
    pub fn regenerate_key(&self) -> Result<(), ProtectError> {
        self.keys.delete()?;
        self.keys.insert_if_absent(SecretKey::generate()?)?;
        Ok(())
    }
}
impl lingopop_core::secrets::SecretProtector for KeyStoreProtector {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
        let key = match self.keys.load()? {
            Some(key) => key,
            None => self.keys.insert_if_absent(SecretKey::generate()?)?,
        };
        seal(&key, plain)
    }
    fn unprotect(&self, bytes: &[u8]) -> Result<Vec<u8>, ProtectError> {
        if bytes.len() < 32 || !bytes.starts_with(MAGIC) {
            return Err(ProtectError::Invalid);
        }
        let key = self.keys.load()?.ok_or(ProtectError::KeyMissing)?;
        open(&key, bytes)
    }
}
