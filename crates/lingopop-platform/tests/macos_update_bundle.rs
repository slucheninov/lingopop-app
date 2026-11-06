//! `SystemBundleTools` against the real `/usr/bin/codesign`, `/usr/bin/ditto`, `/usr/bin/lipo`
//! and `renamex_np(RENAME_SWAP)` on fake bundles in temp dirs, signed ad hoc (no keychain, no
//! network). Runs by default on macOS.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::Path;
use std::process::Command;

use lingopop_platform::update_install::{
    bundle_support, pinned_requirement, BundleTools, Support, BUNDLE_ID,
};
use lingopop_platform::SystemBundleTools;
use tempfile::TempDir;

fn signed_bundle(app: &Path, version: &str) {
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::write(
        contents.join("Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>CFBundleExecutable</key>\n\t<string>LingoPop</string>\n\
             \t<key>CFBundleIdentifier</key>\n\t<string>{BUNDLE_ID}</string>\n\
             \t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n\
             \t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n\
             </dict>\n</plist>\n"
        ),
    )
    .unwrap();
    // A real Mach-O (x86_64 + arm64e on every current macOS) as the main executable.
    fs::copy("/usr/bin/true", contents.join("MacOS/LingoPop")).unwrap();
    let status = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(app)
        .output()
        .unwrap()
        .status;
    assert!(status.success(), "ad-hoc signing");
}

#[test]
fn real_rename_swap_exchanges_two_folders() {
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("f"), b"A").unwrap();
    fs::write(b.join("f"), b"B").unwrap();
    SystemBundleTools.swap(&a, &b).unwrap();
    assert_eq!(fs::read(a.join("f")).unwrap(), b"B");
    assert_eq!(fs::read(b.join("f")).unwrap(), b"A");
    // a missing side is an error and moves nothing
    assert!(SystemBundleTools
        .swap(&a, &tmp.path().join("missing"))
        .is_err());
    assert_eq!(fs::read(a.join("f")).unwrap(), b"B");
}

#[test]
fn real_codesign_reads_the_adhoc_requirement() {
    let tmp = TempDir::new().unwrap();
    let app = tmp.path().join("Applications/LingoPop.app");
    signed_bundle(&app, "0.2.1");
    let dr = SystemBundleTools
        .designated_requirement(&app)
        .expect("a designated requirement");
    assert!(dr.starts_with("cdhash H\""), "ad-hoc DR is a cdhash");
    assert_ne!(dr, pinned_requirement());
    // an ad-hoc running app never self-updates
    assert!(matches!(
        bundle_support(&app, &SystemBundleTools),
        Support::Unsupported(_)
    ));
    // an unsigned folder has no requirement
    let bare = tmp.path().join("Bare.app");
    fs::create_dir_all(&bare).unwrap();
    assert_eq!(SystemBundleTools.designated_requirement(&bare), None);
}

#[test]
fn real_verify_rejects_the_pinned_requirement_for_adhoc() {
    let tmp = TempDir::new().unwrap();
    let app = tmp.path().join("LingoPop.app");
    signed_bundle(&app, "0.2.2");
    assert!(!SystemBundleTools.verify(&app, &pinned_requirement()));
    // the same call accepts a requirement the ad-hoc bundle does satisfy
    assert!(SystemBundleTools.verify(&app, &format!("identifier \"{BUNDLE_ID}\"")));
    // a modified bundle fails the strict deep verification
    fs::write(app.join("Contents/MacOS/LingoPop"), b"tampered").unwrap();
    assert!(!SystemBundleTools.verify(&app, &format!("identifier \"{BUNDLE_ID}\"")));
}

#[test]
fn real_ditto_extracts_and_the_bundle_is_read() {
    let tmp = TempDir::new().unwrap();
    let app = tmp.path().join("src/LingoPop.app");
    signed_bundle(&app, "0.2.2");
    let zip = tmp.path().join("download.zip");
    let status = Command::new("/usr/bin/ditto")
        .args(["-c", "-k", "--norsrc", "--keepParent"])
        .arg(&app)
        .arg(&zip)
        .status()
        .unwrap();
    assert!(status.success());
    let into = tmp.path().join("extracted");
    fs::create_dir(&into).unwrap();
    SystemBundleTools.extract_zip(&zip, &into).unwrap();
    let names: Vec<_> = fs::read_dir(&into)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("LingoPop.app")]);
    let out = into.join("LingoPop.app");
    assert_eq!(
        SystemBundleTools.bundle_info(&out),
        Some((BUNDLE_ID.to_owned(), "0.2.2".to_owned()))
    );
    assert!(SystemBundleTools
        .executable_archs(&out)
        .contains(&"x86_64".to_owned()));
    assert!(SystemBundleTools.verify(&out, &format!("identifier \"{BUNDLE_ID}\"")));
    // not a zip = an install error
    let junk = tmp.path().join("junk.zip");
    fs::write(&junk, b"not a zip").unwrap();
    let into2 = tmp.path().join("extracted2");
    fs::create_dir(&into2).unwrap();
    assert!(SystemBundleTools.extract_zip(&junk, &into2).is_err());
}
