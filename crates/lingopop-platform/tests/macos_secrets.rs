use lingopop_core::secrets::{ProtectError, SecretProtector};
use lingopop_platform::secrets_aead::{open, seal, SecretKey};
#[test]
fn aead_roundtrip_uses_fresh_nonce() {
    let key = SecretKey::generate().unwrap();
    let plain = b"FAKE-SECRET-SENTINEL";
    let a = seal(&key, plain).unwrap();
    let b = seal(&key, plain).unwrap();
    assert_ne!(a, b);
    assert!(!a.windows(plain.len()).any(|w| w == plain));
    assert_eq!(open(&key, &a).unwrap(), plain);
    assert!(!format!("{key:?}").contains("SENTINEL"));
}
#[test]
fn aead_tamper_wrong_key_or_unknown_magic_is_invalid() {
    let key = SecretKey::generate().unwrap();
    let a = seal(&key, b"sentinel").unwrap();
    assert!(matches!(
        open(&SecretKey::generate().unwrap(), &a),
        Err(ProtectError::Invalid)
    ));
    for i in 0..a.len() {
        let mut b = a.clone();
        b[i] ^= 1;
        assert!(matches!(open(&key, &b), Err(ProtectError::Invalid)));
    }
    for i in 0..a.len() {
        assert!(open(&key, &a[..i]).is_err());
    }
}

use lingopop_core::{
    fsio::LoadOutcome, provider::ProviderType, secrets::SecretStore, time::SystemClock,
};
use lingopop_platform::secrets_aead::{KeyStoreProtector, KeychainKeyStore};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
#[derive(Default)]
struct FakeStore {
    key: Mutex<Option<SecretKey>>,
    unavailable: bool,
    inserts: AtomicUsize,
}
impl KeychainKeyStore for FakeStore {
    fn load(&self) -> Result<Option<SecretKey>, ProtectError> {
        if self.unavailable {
            Err(ProtectError::Unavailable)
        } else {
            Ok(self.key.lock().unwrap().clone())
        }
    }
    fn insert_if_absent(&self, key: SecretKey) -> Result<SecretKey, ProtectError> {
        self.inserts.fetch_add(1, Ordering::SeqCst);
        if self.unavailable {
            return Err(ProtectError::Unavailable);
        }
        Ok(self.key.lock().unwrap().get_or_insert(key).clone())
    }
    fn delete(&self) -> Result<(), ProtectError> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }
}
#[test]
fn unavailable_keychain_never_replaces_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.dat");
    let before = seal(&SecretKey::generate().unwrap(), b"{}").unwrap();
    std::fs::write(&path, &before).unwrap();
    let keys = Arc::new(FakeStore {
        unavailable: true,
        ..Default::default()
    });
    let (mut store, outcome) = SecretStore::open(
        path.clone(),
        Arc::new(KeyStoreProtector::new(keys.clone())),
        Arc::new(SystemClock),
    )
    .unwrap();
    assert_eq!(outcome, LoadOutcome::Locked);
    assert!(store.set_api_key(ProviderType::Claude, "sentinel").is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(keys.inserts.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn concurrent_first_launch_returns_one_durable_key() {
    let keys = Arc::new(FakeStore::default());
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let keys = keys.clone();
            std::thread::spawn(move || KeyStoreProtector::new(keys).protect(b"sentinel").unwrap())
        })
        .collect();
    let protector = KeyStoreProtector::new(keys);
    for h in handles {
        assert_eq!(
            protector.unprotect(&h.join().unwrap()).unwrap(),
            b"sentinel"
        );
    }
}
#[test]
fn failed_key_save_writes_no_secrets_file() {
    struct Refuse;
    impl KeychainKeyStore for Refuse {
        fn load(&self) -> Result<Option<SecretKey>, ProtectError> {
            Ok(None)
        }
        fn insert_if_absent(&self, _: SecretKey) -> Result<SecretKey, ProtectError> {
            Err(ProtectError::Unavailable)
        }
        fn delete(&self) -> Result<(), ProtectError> {
            Err(ProtectError::Unavailable)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.dat");
    let (mut store, _) = SecretStore::open(
        path.clone(),
        Arc::new(KeyStoreProtector::new(Arc::new(Refuse))),
        Arc::new(SystemClock),
    )
    .unwrap();
    assert!(store.set_api_key(ProviderType::Claude, "sentinel").is_err());
    assert!(!path.exists());
}

#[test]
#[cfg(target_os = "macos")]
fn native_mac_protector_is_not_identity() {
    // Invalid bytes are rejected before Keychain access; no real item is touched.
    let result = lingopop_platform::native()
        .protector
        .unprotect(b"plain sentinel");
    assert!(matches!(result, Err(ProtectError::Invalid)));
}
#[test]
#[cfg(target_os = "macos")]
#[ignore = "owner-run: touches the login keychain"]
fn native_throwaway_keychain_roundtrip() {
    let keys = lingopop_platform::LoginKeyStore::for_service(format!(
        "app.lingopop.tests.task7.{}",
        std::process::id()
    ));
    struct Cleanup<'a>(&'a dyn KeychainKeyStore);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.delete();
        }
    }
    let _cleanup = Cleanup(&keys);
    let key = keys
        .insert_if_absent(SecretKey::generate().unwrap())
        .unwrap();
    let stored = keys.load().unwrap().unwrap();
    assert_eq!(
        open(&stored, &seal(&key, b"sentinel").unwrap()).unwrap(),
        b"sentinel"
    );
    let other = keys
        .insert_if_absent(SecretKey::generate().unwrap())
        .unwrap();
    assert!(open(&other, &seal(&key, b"sentinel").unwrap()).is_ok());
}

#[test]
fn reset_quarantines_file_and_regenerates_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.dat");
    let keys = Arc::new(FakeStore::default());
    let protector = Arc::new(KeyStoreProtector::new(keys.clone()));
    let (mut store, _) =
        SecretStore::open(path.clone(), protector.clone(), Arc::new(SystemClock)).unwrap();
    store.set_api_key(ProviderType::Claude, "sentinel").unwrap();
    let before = std::fs::read(&path).unwrap();
    store.reset(&SystemClock).unwrap();
    assert!(!store.is_locked());
    assert!(!store.has_api_key(ProviderType::Claude));
    assert!(!path.exists());
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(backup).unwrap(), before);
    assert!(protector.unprotect(&before).is_err());
    store
        .set_api_key(ProviderType::Claude, "new sentinel")
        .unwrap();
    assert_eq!(keys.inserts.load(Ordering::SeqCst), 2);
}

#[test]
fn key_disappearing_after_open_never_generates_replacement() {
    let keys = Arc::new(FakeStore::default());
    let protector = KeyStoreProtector::new(keys.clone());
    let sealed = protector.protect(b"sentinel").unwrap();
    protector.unprotect(&sealed).unwrap();
    *keys.key.lock().unwrap() = None;
    assert!(matches!(
        protector.protect(b"new sentinel"),
        Err(ProtectError::KeyMissing)
    ));
    assert_eq!(keys.inserts.load(Ordering::SeqCst), 1);
}
#[test]
fn protector_error_text_and_debug_are_fixed() {
    for e in [
        ProtectError::Invalid,
        ProtectError::Unavailable,
        ProtectError::KeyMissing,
    ] {
        assert!(!format!("{e} {e:?}").contains("sentinel"));
    }
    let key = SecretKey::from_bytes(&[65; 32]).unwrap();
    assert_eq!(format!("{key:?}"), "SecretKey(<redacted>)");
}

#[test]
fn failed_reset_keeps_store_locked_and_backup_intact() {
    struct FailReset(FakeStore);
    impl KeychainKeyStore for FailReset {
        fn load(&self) -> Result<Option<SecretKey>, ProtectError> {
            self.0.load()
        }
        fn insert_if_absent(&self, key: SecretKey) -> Result<SecretKey, ProtectError> {
            self.0.insert_if_absent(key)
        }
        fn delete(&self) -> Result<(), ProtectError> {
            Err(ProtectError::Unavailable)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.dat");
    let (mut store, _) = SecretStore::open(
        path.clone(),
        Arc::new(KeyStoreProtector::new(Arc::new(FailReset(
            FakeStore::default(),
        )))),
        Arc::new(SystemClock),
    )
    .unwrap();
    store.set_api_key(ProviderType::Claude, "sentinel").unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(store.reset(&SystemClock).is_err());
    assert!(store.is_locked());
    assert!(!store.has_api_key(ProviderType::Claude));
    assert!(store
        .set_api_key(ProviderType::Claude, "new sentinel")
        .is_err());
    assert!(!path.exists());
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(backup).unwrap(), before);
}
#[test]
fn existing_ciphertext_with_missing_key_stays_locked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.dat");
    let before = seal(&SecretKey::generate().unwrap(), b"{}").unwrap();
    std::fs::write(&path, &before).unwrap();
    let keys = Arc::new(FakeStore::default());
    let (_, outcome) = SecretStore::open(
        path.clone(),
        Arc::new(KeyStoreProtector::new(keys.clone())),
        Arc::new(SystemClock),
    )
    .unwrap();
    assert_eq!(outcome, LoadOutcome::Locked);
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(keys.inserts.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
