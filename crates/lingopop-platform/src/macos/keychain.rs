//! Legacy file keychain. Never request the data-protection keychain or an access group.
use crate::secrets_aead::{KeychainKeyStore, SecretKey};
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::CFDictionary,
    string::{CFString, CFStringRef},
};
use lingopop_core::secrets::ProtectError;
use security_framework::passwords::{
    delete_generic_password_options, generic_password, PasswordOptions,
};
use security_framework_sys::{
    access_control::kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, item::*,
    keychain_item::SecItemAdd,
};
use zeroize::Zeroizing;

// The high-level crate has no accessibility setter; this SDK constant is missing from sys.
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecAttrAccessible: CFStringRef;
}

pub struct LoginKeyStore {
    service: String,
}
impl Default for LoginKeyStore {
    fn default() -> Self {
        Self {
            service: "app.lingopop.tauri.macos".into(),
        }
    }
}
impl LoginKeyStore {
    pub fn for_service(service: String) -> Self {
        Self { service }
    }
    fn options(&self) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(&self.service, "secrets-key-v1");
        options.set_access_synchronized(Some(false));
        options
    }
}
impl KeychainKeyStore for LoginKeyStore {
    fn load(&self) -> Result<Option<SecretKey>, ProtectError> {
        match generic_password(self.options()) {
            Ok(bytes) => SecretKey::from_bytes(&Zeroizing::new(bytes)).map(Some),
            Err(e) if e.code() == -25300 => Ok(None),
            Err(_) => Err(ProtectError::Unavailable),
        }
    }
    fn insert_if_absent(&self, key: SecretKey) -> Result<SecretKey, ProtectError> {
        // SAFETY: Security framework constants are valid immortal CFStrings. The dictionary
        // owns references to all values and remains alive throughout SecItemAdd. No result requested.
        let status = unsafe {
            let pair =
                |name: CFStringRef, value: CFType| (CFString::wrap_under_get_rule(name), value);
            let pairs = [
                pair(
                    kSecClass,
                    CFString::wrap_under_get_rule(kSecClassGenericPassword).into_CFType(),
                ),
                pair(kSecAttrService, CFString::new(&self.service).into_CFType()),
                pair(
                    kSecAttrAccount,
                    CFString::new("secrets-key-v1").into_CFType(),
                ),
                pair(
                    kSecAttrSynchronizable,
                    CFBoolean::false_value().into_CFType(),
                ),
                pair(
                    kSecAttrAccessible,
                    CFString::wrap_under_get_rule(kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly)
                        .into_CFType(),
                ),
                pair(
                    kSecValueData,
                    CFData::from_buffer(key.as_bytes()).into_CFType(),
                ),
            ];
            let dict = CFDictionary::from_CFType_pairs(&pairs);
            SecItemAdd(dict.as_concrete_TypeRef(), std::ptr::null_mut())
        };
        match status {
            0 => Ok(key),
            -25299 => self.load()?.ok_or(ProtectError::Unavailable),
            _ => Err(ProtectError::Unavailable),
        }
    }
    fn delete(&self) -> Result<(), ProtectError> {
        match delete_generic_password_options(self.options()) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(_) => Err(ProtectError::Unavailable),
        }
    }
}
