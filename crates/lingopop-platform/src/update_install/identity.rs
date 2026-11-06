//! Item identity (spec section 3, "Identity, not position"): Windows = the SHA-256 of the exe;
//! macOS = `<CFBundleShortVersionString>:<SHA-256 of Contents/_CodeSignature/CodeResources>:
//! <SHA-256 of the main executable>` (the executable named by `CFBundleExecutable`). `None`
//! means "unknown": nothing readable, never a match.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{to_hex, SiteKind};

/// Lowercase hex SHA-256 of a regular file; `None` when it is missing, not a regular file
/// (symlinks are followed) or unreadable.
pub fn file_sha256(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    hash_file(path).ok()
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// The identity of a `.app` bundle (see the module doc). The `Info.plist` must be an XML plist
/// (what the Tauri bundler writes); a binary plist, a missing key or an executable name that is
/// not a plain file name gives `None`.
pub fn bundle_identity(app: &Path) -> Option<String> {
    let contents = app.join("Contents");
    let plist = std::fs::read(contents.join("Info.plist")).ok()?;
    let plist = std::str::from_utf8(&plist).ok()?;
    let version = plist_string(plist, "CFBundleShortVersionString")?;
    let executable = plist_string(plist, "CFBundleExecutable")?;
    if executable.is_empty()
        || executable == "."
        || executable == ".."
        || executable.contains(['/', '\\', '\0'])
    {
        return None;
    }
    let resources = file_sha256(&contents.join("_CodeSignature").join("CodeResources"))?;
    let main = file_sha256(&contents.join("MacOS").join(&executable))?;
    Some(format!("{version}:{resources}:{main}"))
}

/// The identity function recovery uses for a site kind.
pub fn identity_for(kind: SiteKind) -> fn(&Path) -> Option<String> {
    match kind {
        SiteKind::WindowsExe => file_sha256,
        SiteKind::MacBundle => bundle_identity,
    }
}

/// The `<string>` value right after `<key>KEY</key>` in an XML plist (whitespace allowed in
/// between; `<string/>` is the empty string). Minimal by design: it reads the first occurrence
/// of the key, which for `CFBundle*` keys is the top-level one in every bundle plist.
pub(crate) fn plist_string(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let rest = &xml[xml.find(&needle)? + needle.len()..];
    let rest = rest.trim_start();
    if rest.starts_with("<string/>") {
        return Some(String::new());
    }
    let rest = rest.strip_prefix("<string>")?;
    let end = rest.find("</string>")?;
    unescape(&rest[..end])
}

fn unescape(s: &str) -> Option<String> {
    if s.contains('<') {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';')?;
        out.push(match &rest[..=end] {
            "&amp;" => '&',
            "&lt;" => '<',
            "&gt;" => '>',
            "&quot;" => '"',
            "&apos;" => '\'',
            _ => return None,
        });
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_string_reads_values_and_entities() {
        let xml =
            "<dict>\n\t<key>A</key>\n\t<string>1.2&amp;3</string>\n<key>B</key><string/></dict>";
        assert_eq!(plist_string(xml, "A").as_deref(), Some("1.2&3"));
        assert_eq!(plist_string(xml, "B").as_deref(), Some(""));
        assert_eq!(plist_string(xml, "C"), None);
        assert_eq!(plist_string("<key>A</key><integer>1</integer>", "A"), None);
        assert_eq!(
            plist_string("<key>A</key><string>x&bogus;</string>", "A"),
            None
        );
    }

    #[test]
    fn hex_sha256_of_a_known_value() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            file_sha256(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(file_sha256(dir.path()), None);
        assert_eq!(file_sha256(&dir.path().join("missing")), None);
    }
}
