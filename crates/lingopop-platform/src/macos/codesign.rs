//! `SystemBundleTools`: the macOS [`BundleTools`] over `/usr/bin/codesign`, `/usr/bin/ditto`,
//! `/usr/bin/lipo` (each through `std::process::Command` with an argument list, no shell, no
//! stdin) and `renamex_np(RENAME_SWAP)` from `macos/fs.rs`. Tool output is parsed, never
//! logged; only absolute paths are passed, so no path can be read as an option.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::update_install::{bundle_executable, read_bundle_info, BundleTools, UpdateError};

const CODESIGN: &str = "/usr/bin/codesign";
const DITTO: &str = "/usr/bin/ditto";
const LIPO: &str = "/usr/bin/lipo";

/// The production bundle tools.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBundleTools;

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Option<Output> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()
}

/// The text after `designated => ` in `codesign -d -r-` output (an implicit ad-hoc requirement
/// is printed as a `# designated => ...` comment; it is read the same way).
pub(crate) fn parse_designated(stdout: &str) -> Option<String> {
    const MARK: &str = "designated => ";
    stdout.lines().find_map(|line| {
        let at = line.find(MARK)?;
        let text = line[at + MARK.len()..].trim_end();
        (!text.is_empty()).then(|| text.to_owned())
    })
}

impl BundleTools for SystemBundleTools {
    fn designated_requirement(&self, app: &Path) -> Option<String> {
        if !app.is_absolute() {
            return None;
        }
        let out = run(CODESIGN, &["-d".as_ref(), "-r-".as_ref(), app.as_os_str()])?;
        if !out.status.success() {
            return None;
        }
        parse_designated(std::str::from_utf8(&out.stdout).ok()?)
    }

    fn verify(&self, app: &Path, requirement: &str) -> bool {
        if !app.is_absolute() {
            return false;
        }
        let req = format!("-R={requirement}");
        run(
            CODESIGN,
            &[
                "--verify".as_ref(),
                "--strict".as_ref(),
                "--deep".as_ref(),
                req.as_ref(),
                app.as_os_str(),
            ],
        )
        .is_some_and(|out| out.status.success())
    }

    fn extract_zip(&self, zip: &Path, into: &Path) -> Result<(), UpdateError> {
        const FAILED: UpdateError = UpdateError::Install("update could not be extracted");
        if !zip.is_absolute() || !into.is_absolute() {
            return Err(FAILED);
        }
        let out = run(
            DITTO,
            &[
                "-x".as_ref(),
                "-k".as_ref(),
                "--noqtn".as_ref(),
                zip.as_os_str(),
                into.as_os_str(),
            ],
        )
        .ok_or(FAILED)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(FAILED)
        }
    }

    fn bundle_info(&self, app: &Path) -> Option<(String, String)> {
        read_bundle_info(app)
    }

    fn executable_archs(&self, app: &Path) -> Vec<String> {
        let Some(exe) = bundle_executable(app).filter(|p| p.is_absolute()) else {
            return Vec::new();
        };
        match run(LIPO, &["-archs".as_ref(), exe.as_os_str()]) {
            Some(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn swap(&self, a: &Path, b: &Path) -> Result<(), UpdateError> {
        super::fs::rename_swap(a, b).map_err(|_| UpdateError::Install("bundle swap failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn designated_requirement_is_the_text_after_the_arrow() {
        assert_eq!(
            parse_designated("# designated => cdhash H\"ab\" or cdhash H\"cd\"\n").as_deref(),
            Some("cdhash H\"ab\" or cdhash H\"cd\"")
        );
        assert_eq!(
            parse_designated("designated => identifier \"x\" and certificate leaf = H\"0011\"\n")
                .as_deref(),
            Some("identifier \"x\" and certificate leaf = H\"0011\"")
        );
        assert_eq!(parse_designated("Executable=/x\n"), None);
        assert_eq!(parse_designated("designated => \n"), None);
    }
}
