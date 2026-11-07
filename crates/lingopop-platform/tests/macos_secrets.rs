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
    loads: AtomicUsize,
    inserts: AtomicUsize,
}
impl KeychainKeyStore for FakeStore {
    fn load(&self) -> Result<Option<SecretKey>, ProtectError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
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
fn loaded_key_is_cached_and_never_replaced_when_keychain_item_disappears() {
    let keys = Arc::new(FakeStore::default());
    let protector = KeyStoreProtector::new(keys.clone());
    let sealed = protector.protect(b"sentinel").unwrap();
    protector.unprotect(&sealed).unwrap();
    *keys.key.lock().unwrap() = None;
    let next = protector.protect(b"new sentinel").unwrap();
    assert_eq!(protector.unprotect(&next).unwrap(), b"new sentinel");
    assert_eq!(keys.inserts.load(Ordering::SeqCst), 1);
    assert_eq!(keys.loads.load(Ordering::SeqCst), 1);
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

/// Key store whose first insert after a delete can be made to fail.
#[derive(Default)]
struct RotationStore {
    key: Mutex<Option<SecretKey>>,
    fail_insert_once: std::sync::atomic::AtomicBool,
    unreadable: bool,
    deletes: AtomicUsize,
}
impl KeychainKeyStore for RotationStore {
    fn load(&self) -> Result<Option<SecretKey>, ProtectError> {
        if self.unreadable {
            return Err(ProtectError::Unavailable);
        }
        Ok(self.key.lock().unwrap().clone())
    }
    fn insert_if_absent(&self, key: SecretKey) -> Result<SecretKey, ProtectError> {
        if self.fail_insert_once.swap(false, Ordering::SeqCst) {
            return Err(ProtectError::Unavailable);
        }
        Ok(self.key.lock().unwrap().get_or_insert(key).clone())
    }
    fn delete(&self) -> Result<(), ProtectError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        *self.key.lock().unwrap() = None;
        Ok(())
    }
}

#[test]
fn key_generation_failure_deletes_nothing() {
    let keys = Arc::new(RotationStore::default());
    let old = SecretKey::generate().unwrap();
    *keys.key.lock().unwrap() = Some(old.clone());
    let protector =
        KeyStoreProtector::new(keys.clone()).with_generator(|| Err(ProtectError::Unavailable));
    assert!(protector.regenerate_key().is_err());
    assert_eq!(keys.deletes.load(Ordering::SeqCst), 0);
    let kept = keys.key.lock().unwrap().clone().unwrap();
    assert_eq!(kept.as_bytes(), old.as_bytes());
}

#[test]
fn insert_failure_after_delete_restores_the_old_key() {
    let keys = Arc::new(RotationStore::default());
    let old = SecretKey::generate().unwrap();
    *keys.key.lock().unwrap() = Some(old.clone());
    keys.fail_insert_once.store(true, Ordering::SeqCst);
    let protector = KeyStoreProtector::new(keys.clone());
    assert!(matches!(
        protector.regenerate_key(),
        Err(ProtectError::Unavailable)
    ));
    assert_eq!(keys.deletes.load(Ordering::SeqCst), 1);
    let restored = keys.key.lock().unwrap().clone().unwrap();
    assert_eq!(restored.as_bytes(), old.as_bytes());
}

#[test]
fn insert_failure_with_unreadable_old_key_reports_the_insert_error() {
    let keys = Arc::new(RotationStore {
        unreadable: true,
        ..Default::default()
    });
    *keys.key.lock().unwrap() = Some(SecretKey::generate().unwrap());
    keys.fail_insert_once.store(true, Ordering::SeqCst);
    let protector = KeyStoreProtector::new(keys.clone());
    assert!(protector.regenerate_key().is_err());
    assert!(keys.key.lock().unwrap().is_none());
}

/// A blob sealed by aes-gcm 0.10.3 (the 0.2.1 release) still opens: the LPS1
/// format is unchanged by a library upgrade. Test key 0x42 * 32, not a real key.
#[test]
fn aead_opens_a_blob_sealed_by_the_previous_library() {
    const BLOB: &str = "4c50533132f38c5a2d405bfe8d56d42abb48299f693e8c8d85fc78c31aeb64239e4e3abf098b5bbb3f41f42949aeae29d8e57ed587877d6feb80175656ae8227c263";
    let bytes: Vec<u8> = (0..BLOB.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&BLOB[i..i + 2], 16).unwrap())
        .collect();
    let key = SecretKey::from_bytes(&[0x42; 32]).unwrap();
    let plain = open(&key, &bytes).unwrap();
    assert_eq!(plain, b"{\"apiKey.Claude\":\"test-not-a-key\"}");
}
