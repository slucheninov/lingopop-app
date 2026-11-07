//! Release tooling for LingoPop update assets.
//!
//! ```text
//! lingopop-release-tools keygen --secret <file> --public <file> --password-env <VAR>
//! lingopop-release-tools sign --key <file> --password-env <VAR> <asset>...
//! lingopop-release-tools verify --public-key <file|const> <asset>...
//! ```
//!
//! The password is read from the named environment variable and never printed.

use std::fs;
use std::io::{self, Cursor};
use std::path::Path;
use std::process::ExitCode;

use lingopop_core::update_sig::{update_comment, verify_update_with};
use lingopop_core::updates::UPDATE_PUBLIC_KEY;

/// Every failure of the tool. The messages name paths, options and variable names only, never a
/// password or key material (a failed key load has one fixed message).
#[derive(Debug, thiserror::Error)]
enum ToolError {
    #[error("usage: lingopop-release-tools sign|verify|keygen ...")]
    Usage,
    #[error("unknown option --{0}")]
    UnknownOption(String),
    #[error("--{0} needs a value")]
    MissingValue(String),
    #[error("--{0} is required")]
    Required(&'static str),
    #[error("environment variable {0} is not set")]
    MissingEnv(String),
    #[error("no asset given")]
    NoAsset,
    #[error("{0}: not a file path")]
    NotAFilePath(String),
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("cannot load the secret key (wrong password or bad file)")]
    SecretKey,
    #[error("{0}: signing failed")]
    Signing(String),
    #[error("unrecognized public key file")]
    PublicKeyFile,
    #[error("{0}")]
    Signature(String),
    #[error("verification failed")]
    VerificationFailed,
    #[error("key generation failed")]
    Keygen,
}

fn io_err(path: &str) -> impl FnOnce(io::Error) -> ToolError + '_ {
    move |source| ToolError::Io {
        path: path.to_owned(),
        source,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), ToolError> {
    match args.first().map(String::as_str) {
        Some("sign") => sign(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        _ => Err(ToolError::Usage),
    }
}

/// Splits `--name value` options from positional arguments.
fn parse(args: &[String], names: &[&str]) -> Result<(Vec<Option<String>>, Vec<String>), ToolError> {
    let mut values = vec![None; names.len()];
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            let i = names
                .iter()
                .position(|n| *n == name)
                .ok_or_else(|| ToolError::UnknownOption(name.to_owned()))?;
            values[i] = Some(
                it.next()
                    .ok_or_else(|| ToolError::MissingValue(name.to_owned()))?
                    .clone(),
            );
        } else {
            rest.push(a.clone());
        }
    }
    Ok((values, rest))
}

fn need(v: &Option<String>, name: &'static str) -> Result<String, ToolError> {
    v.clone().ok_or(ToolError::Required(name))
}

fn password(var: &str) -> Result<String, ToolError> {
    std::env::var(var).map_err(|_| ToolError::MissingEnv(var.to_owned()))
}

fn basename(path: &str) -> Result<String, ToolError> {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| ToolError::NotAFilePath(path.to_owned()))
}

fn sign(args: &[String]) -> Result<(), ToolError> {
    let (v, assets) = parse(args, &["key", "password-env"])?;
    let key_path = need(&v[0], "key")?;
    let pw = password(&need(&v[1], "password-env")?)?;
    if assets.is_empty() {
        return Err(ToolError::NoAsset);
    }
    let sk =
        minisign::SecretKey::from_file(&key_path, Some(pw)).map_err(|_| ToolError::SecretKey)?;
    for asset in &assets {
        let data = fs::read(asset).map_err(io_err(asset))?;
        let comment = update_comment(&basename(asset)?);
        let sig = minisign::sign(None, &sk, Cursor::new(data), Some(&comment), None)
            .map_err(|_| ToolError::Signing(asset.clone()))?;
        let sig_path = format!("{asset}.minisig");
        fs::write(&sig_path, sig.into_string()).map_err(io_err(&sig_path))?;
        println!("signed {asset}");
    }
    Ok(())
}

/// The base64 key line of a minisign `.pub` file (second line), or of a bare key line.
fn key_line(text: &str) -> Result<String, ToolError> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    match lines.as_slice() {
        [one] => Ok((*one).to_string()),
        [first, second, ..] if first.starts_with("untrusted comment:") => Ok((*second).to_string()),
        _ => Err(ToolError::PublicKeyFile),
    }
}

fn verify(args: &[String]) -> Result<(), ToolError> {
    let (v, assets) = parse(args, &["public-key"])?;
    let pk = need(&v[0], "public-key")?;
    if assets.is_empty() {
        return Err(ToolError::NoAsset);
    }
    let key = if pk == "const" {
        UPDATE_PUBLIC_KEY.to_string()
    } else {
        key_line(&fs::read_to_string(&pk).map_err(io_err(&pk))?)?
    };
    let mut failed = false;
    for asset in &assets {
        let result = (|| -> Result<(), ToolError> {
            let data = fs::read(asset).map_err(io_err(asset))?;
            let sig_path = format!("{asset}.minisig");
            let sig = fs::read_to_string(&sig_path).map_err(io_err(&sig_path))?;
            verify_update_with(&key, &data, &sig, &update_comment(&basename(asset)?))
                .map_err(|e| ToolError::Signature(e.to_string()))
        })();
        match result {
            Ok(()) => println!("ok {asset}"),
            Err(e) => {
                eprintln!("FAILED {asset}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        Err(ToolError::VerificationFailed)
    } else {
        Ok(())
    }
}

/// Generates a password-protected key pair (used for the committed TEST fixtures).
fn keygen(args: &[String]) -> Result<(), ToolError> {
    let (v, _) = parse(args, &["secret", "public", "password-env"])?;
    let secret = need(&v[0], "secret")?;
    let public = need(&v[1], "public")?;
    let pw = password(&need(&v[2], "password-env")?)?;
    let sk = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&secret)
        .map_err(io_err(&secret))?;
    let pk = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&public)
        .map_err(io_err(&public))?;
    minisign::KeyPair::generate_and_write_encrypted_keypair(pk, sk, None, Some(pw))
        .map_err(|_| ToolError::Keygen)?;
    Ok(())
}
