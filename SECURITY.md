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

- In-app updates (from 0.2.1) install only files signed with the project's minisign (Ed25519) update key: the public key is built into the app, each signature names the exact asset it belongs to, and the app also checks the download host, the size and the release version. On macOS the new bundle must additionally satisfy the installed app's designated requirement (bundle id plus the release certificate), so a file from another signer is refused. When a check fails before the swap, nothing is installed. On macOS the new bundle is checked again after the swap; if that late check fails, the previous version is swapped back at once, or, should that swap back fail too, at the next start. The private key and its password are repository secrets used only by the publish job of a tag run.
- The Windows exe is not code-signed, so SmartScreen warns on first launch.
- Tag releases of the macOS app are signed with one long-lived self-signed code-signing certificate (hardened runtime, not notarized, so Gatekeeper asks you to confirm the first launch). The certificate is not trusted by the system: it exists so macOS keeps your permissions across updates and so the updater can tell a genuine update, not to prove the publisher. Test builds without the certificate are signed ad hoc. Either way Gatekeeper asks you to confirm the first launch. The release workflow also supports Developer ID signing and notarization; the notes of such a release say so and skip the Gatekeeper steps.

Download LingoPop only from this repository's [Releases page](https://github.com/slucheninov/lingopop-app/releases).
