use lingopop_core::secrets::ProtectError;
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
