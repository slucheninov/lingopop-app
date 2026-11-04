//! `DevPlaintextProtector`: the identity "protector" used by the non-Windows dev builds.

use lingopop_core::secrets::{ProtectError, SecretProtector};

/// NOT FOR SHIPPING: stores `secrets.dat` as PLAINTEXT (identity "encryption").
///
/// Available only for tests or an explicit dev-plaintext feature; native() never selects it.
#[derive(Debug, Default, Clone, Copy)]
pub struct DevPlaintextProtector;

impl DevPlaintextProtector {
    pub fn new() -> Self {
        Self
    }
}

impl SecretProtector for DevPlaintextProtector {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(plain.to_vec())
    }

    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_protector_roundtrips_bytes() {
        let p = DevPlaintextProtector::new();
        for plain in [
            b"".as_slice(),
            "{\"apiKey.Claude\":\"k\",\"n\":\"Привет\"}".as_bytes(),
        ] {
            let sealed = p.protect(plain).unwrap();
            assert_eq!(sealed, plain, "identity");
            assert_eq!(p.unprotect(&sealed).unwrap(), plain);
        }
    }
}
