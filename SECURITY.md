# Security Policy

## Supported versions

Only the latest release receives security fixes. Please update to the latest version before reporting an issue.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability** (private vulnerability reporting). Do not open a public issue or pull request for a security problem, and do not include API keys or other secrets in any report.

You can expect an acknowledgement as soon as the maintainer is able to look at the report; fixes ship in a new release.

## How API keys are stored

API keys are stored locally in `%APPDATA%\LingoPop\secrets.dat`. The file is protected with Windows DPAPI (current user scope), so it can only be decrypted by the same Windows user on the same machine. This does not protect against other processes running as the same user. Keys are never synced anywhere. See [PRIVACY.md](PRIVACY.md) for what the app sends over the network.
