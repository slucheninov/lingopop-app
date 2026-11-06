//! Verification of update assets: a minisign (Ed25519, prehashed) signature whose trusted
//! comment names the asset, so a signature for one release asset cannot vouch for another.

use minisign_verify::{PublicKey, Signature};

use crate::updates::UPDATE_PUBLIC_KEY;

/// The trusted comment every update signature must carry for `asset_name`.
pub fn update_comment(asset_name: &str) -> String {
    format!("lingopop-update {asset_name}")
}

/// Why an update could not be fetched, verified or installed. `Display` is fixed English text:
/// no URL, path, payload or OS text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    Signature,
    Comment,
    Download,
    TooLarge,
    Network,
    Unsupported(&'static str),
    Install(&'static str),
    Cancelled,
}

impl UpdateError {
    /// The English catalog key shown to the user (translated by the UI layer).
    pub fn catalog_key(&self) -> &'static str {
        match self {
            UpdateError::Signature | UpdateError::Comment => "The update could not be verified.",
            UpdateError::Download
            | UpdateError::TooLarge
            | UpdateError::Network
            | UpdateError::Cancelled => "The update could not be downloaded.",
            UpdateError::Install(_) => {
                "The update could not be installed. Your current version is unchanged."
            }
            UpdateError::Unsupported(_) => {
                "LingoPop can't update itself here. Download the new version instead."
            }
        }
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Signature => f.write_str("update signature is invalid"),
            UpdateError::Comment => f.write_str("update signature is for a different asset"),
            UpdateError::Download => f.write_str("update download failed"),
            UpdateError::TooLarge => f.write_str("update is too large"),
            UpdateError::Network => f.write_str("update network request failed"),
            UpdateError::Unsupported(why) => write!(f, "update unsupported: {why}"),
            UpdateError::Install(why) => write!(f, "update install failed: {why}"),
            UpdateError::Cancelled => f.write_str("update cancelled"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// Verify `bytes` against `signature` (the text of a `.minisig` file) with `public_key` (the
/// base64 key line) and require the trusted comment to equal `expected_comment`.
pub fn verify_update_with(
    public_key: &str,
    bytes: &[u8],
    signature: &str,
    expected_comment: &str,
) -> Result<(), UpdateError> {
    let key = PublicKey::from_base64(public_key.trim()).map_err(|_| UpdateError::Signature)?;
    let sig = Signature::decode(signature).map_err(|_| UpdateError::Signature)?;
    key.verify(bytes, &sig, false)
        .map_err(|_| UpdateError::Signature)?;
    if sig.trusted_comment() != expected_comment {
        return Err(UpdateError::Comment);
    }
    Ok(())
}

/// [`verify_update_with`] using the pinned [`UPDATE_PUBLIC_KEY`].
pub fn verify_update(
    bytes: &[u8],
    signature: &str,
    expected_comment: &str,
) -> Result<(), UpdateError> {
    verify_update_with(UPDATE_PUBLIC_KEY, bytes, signature, expected_comment)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSET: &str = "LingoPop-0.2.1-test.zip";
    const BYTES: &[u8] = include_bytes!("../tests/fixtures/updates/LingoPop-0.2.1-test.zip");
    const SIG: &str = include_str!("../tests/fixtures/updates/LingoPop-0.2.1-test.zip.minisig");
    const PUB: &str = include_str!("../tests/fixtures/updates/test.pub");
    const OTHER_PUB: &str = include_str!("../tests/fixtures/updates/other.pub");

    fn key_line(file: &str) -> &str {
        file.lines().nth(1).unwrap().trim()
    }

    #[test]
    fn good_signature_verifies() {
        let r = verify_update_with(key_line(PUB), BYTES, SIG, &update_comment(ASSET));
        assert_eq!(r, Ok(()));
    }

    #[test]
    fn tampered_byte_is_rejected() {
        let mut bytes = BYTES.to_vec();
        bytes[0] ^= 1;
        let r = verify_update_with(key_line(PUB), &bytes, SIG, &update_comment(ASSET));
        assert_eq!(r, Err(UpdateError::Signature));
    }

    #[test]
    fn other_key_is_rejected() {
        let r = verify_update_with(key_line(OTHER_PUB), BYTES, SIG, &update_comment(ASSET));
        assert_eq!(r, Err(UpdateError::Signature));
    }

    #[test]
    fn wrong_comment_is_rejected() {
        let r = verify_update_with(
            key_line(PUB),
            BYTES,
            SIG,
            &update_comment("LingoPop-0.2.2-test.zip"),
        );
        assert_eq!(r, Err(UpdateError::Comment));
    }

    #[test]
    fn garbage_signature_is_rejected() {
        let r = verify_update_with(key_line(PUB), BYTES, "not a signature", "x");
        assert_eq!(r, Err(UpdateError::Signature));
    }

    #[test]
    fn pinned_key_rejects_the_test_signature() {
        assert_eq!(
            verify_update(BYTES, SIG, &update_comment(ASSET)),
            Err(UpdateError::Signature)
        );
    }

    #[test]
    fn catalog_keys_are_fixed() {
        assert_eq!(
            UpdateError::Comment.catalog_key(),
            "The update could not be verified."
        );
        assert_eq!(
            UpdateError::Cancelled.catalog_key(),
            "The update could not be downloaded."
        );
        assert_eq!(
            UpdateError::Install("x").catalog_key(),
            "The update could not be installed. Your current version is unchanged."
        );
        assert_eq!(
            UpdateError::Unsupported("x").catalog_key(),
            "LingoPop can't update itself here. Download the new version instead."
        );
    }
}
