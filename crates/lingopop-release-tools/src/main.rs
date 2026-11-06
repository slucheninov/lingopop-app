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
use std::io::Cursor;
use std::path::Path;
use std::process::ExitCode;

use lingopop_core::update_sig::{update_comment, verify_update_with};
use lingopop_core::updates::UPDATE_PUBLIC_KEY;

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

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("sign") => sign(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        _ => Err("usage: lingopop-release-tools sign|verify|keygen ...".into()),
    }
}

/// Splits `--name value` options from positional arguments.
fn parse(args: &[String], names: &[&str]) -> Result<(Vec<Option<String>>, Vec<String>), String> {
    let mut values = vec![None; names.len()];
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            let i = names
                .iter()
                .position(|n| *n == name)
                .ok_or_else(|| format!("unknown option --{name}"))?;
            values[i] = Some(
                it.next()
                    .ok_or_else(|| format!("--{name} needs a value"))?
                    .clone(),
            );
        } else {
            rest.push(a.clone());
        }
    }
    Ok((values, rest))
}

fn need(v: &Option<String>, name: &str) -> Result<String, String> {
    v.clone().ok_or_else(|| format!("--{name} is required"))
}

fn password(var: &str) -> Result<String, String> {
    std::env::var(var).map_err(|_| format!("environment variable {var} is not set"))
}

fn basename(path: &str) -> Result<String, String> {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{path}: not a file path"))
}

fn sign(args: &[String]) -> Result<(), String> {
    let (v, assets) = parse(args, &["key", "password-env"])?;
    let key_path = need(&v[0], "key")?;
    let pw = password(&need(&v[1], "password-env")?)?;
    if assets.is_empty() {
        return Err("no asset given".into());
    }
    let sk = minisign::SecretKey::from_file(&key_path, Some(pw))
        .map_err(|_| "cannot load the secret key (wrong password or bad file)".to_string())?;
    for asset in &assets {
        let data = fs::read(asset).map_err(|e| format!("{asset}: {e}"))?;
        let comment = update_comment(&basename(asset)?);
        let sig = minisign::sign(None, &sk, Cursor::new(data), Some(&comment), None)
            .map_err(|_| format!("{asset}: signing failed"))?;
        fs::write(format!("{asset}.minisig"), sig.into_string())
            .map_err(|e| format!("{asset}.minisig: {e}"))?;
        println!("signed {asset}");
    }
    Ok(())
}

/// The base64 key line of a minisign `.pub` file (second line), or of a bare key line.
fn key_line(text: &str) -> Result<String, String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    match lines.as_slice() {
        [one] => Ok((*one).to_string()),
        [first, second, ..] if first.starts_with("untrusted comment:") => Ok((*second).to_string()),
        _ => Err("unrecognized public key file".into()),
    }
}

fn verify(args: &[String]) -> Result<(), String> {
    let (v, assets) = parse(args, &["public-key"])?;
    let pk = need(&v[0], "public-key")?;
    if assets.is_empty() {
        return Err("no asset given".into());
    }
    let key = if pk == "const" {
        UPDATE_PUBLIC_KEY.to_string()
    } else {
        key_line(&fs::read_to_string(&pk).map_err(|e| format!("{pk}: {e}"))?)?
    };
    let mut failed = false;
    for asset in &assets {
        let result = (|| {
            let data = fs::read(asset).map_err(|e| e.to_string())?;
            let sig = fs::read_to_string(format!("{asset}.minisig")).map_err(|e| e.to_string())?;
            verify_update_with(&key, &data, &sig, &update_comment(&basename(asset)?))
                .map_err(|e| e.to_string())
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
        Err("verification failed".into())
    } else {
        Ok(())
    }
}

/// Generates a password-protected key pair (used for the committed TEST fixtures).
fn keygen(args: &[String]) -> Result<(), String> {
    let (v, _) = parse(args, &["secret", "public", "password-env"])?;
    let secret = need(&v[0], "secret")?;
    let public = need(&v[1], "public")?;
    let pw = password(&need(&v[2], "password-env")?)?;
    let sk = fs::File::create(&secret).map_err(|e| format!("{secret}: {e}"))?;
    let pk = fs::File::create(&public).map_err(|e| format!("{public}: {e}"))?;
    minisign::KeyPair::generate_and_write_encrypted_keypair(pk, sk, None, Some(pw))
        .map_err(|_| "key generation failed".to_string())?;
    Ok(())
}
