# Security Policy

## Supported versions

Only the latest release receives security fixes. Please update to the latest version before reporting an issue.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability** (private vulnerability reporting). Do not open a public issue or pull request for a security problem, and do not include API keys or other secrets in any report.

You can expect an acknowledgement as soon as the maintainer is able to look at the report; fixes ship in a new release.

## How API keys are stored

API keys and provider settings are stored only on your computer, in `secrets.dat` inside the app's data folder. LingoPop never syncs them (system backups or a roaming profile may copy the data folder), never shows them again after saving and never writes them to logs or error messages.

- **Windows:** `%APPDATA%\LingoPop\secrets.dat`, protected with Windows DPAPI (current user scope), so normally only the same Windows user on the same machine can decrypt it (a roaming profile or a domain DPAPI backup key can also decrypt it).
- **macOS:** `~/Library/Application Support/LingoPop-Tauri/secrets.dat`, an authenticated AES-256-GCM container with a random nonce per write. The 256-bit key is generated randomly and kept in your login keychain (service `app.lingopop.tauri.macos`, non-synchronizable, so it is not copied to iCloud Keychain). The files are written with owner-only permissions.

This protects the keys at rest. It does not protect against other processes running as the same user. See [PRIVACY.md](PRIVACY.md) for what the app sends over the network.

## Release builds

Releases are built by GitHub Actions from a tagged commit on `main`. Every release carries `SHA256SUMS.txt` with the SHA-256 checksum of each file; the workflow verifies the files and the uploaded copies against it before the release is published.

- The Windows exe is not code-signed, so SmartScreen warns on first launch.
- The macOS app is signed ad hoc with the hardened runtime and not notarized (the default), so Gatekeeper asks you to confirm the first launch. The release workflow also supports Developer ID signing and notarization; the notes of such a release say so and skip the Gatekeeper steps.

Download LingoPop only from this repository's [Releases page](https://github.com/slucheninov/lingopop-app/releases).
