# Installing LingoPop

LingoPop is published as pre-release builds on the [Releases page](https://github.com/slucheninov/lingopop-app/releases). Everything below the marker comment is also copied into the notes of every release by the release workflow (`scripts/verify-release-assets.py publish`): `<version>` becomes the release version, the Windows or macOS part is left out when the release has no assets for it, and the Gatekeeper paragraph is left out when the macOS build is notarized. Keep the `<!-- if ... -->` / `<!-- endif -->` markers balanced (the release tests check it).

<!-- release-notes-start -->
## Installation

### Which file to download

<!-- if windows -->
- **Windows 10/11 (x64):** `LingoPop-<version>-windows-x64-portable.exe`
<!-- endif -->
<!-- if macos -->
- **macOS 13 or newer, Apple silicon (M1 and later):** `LingoPop-<version>-macos-arm64.dmg`
- **macOS 13 or newer, Intel:** `LingoPop-<version>-macos-x64.dmg`
- The `.zip` files hold the same app as the `.dmg` of the same architecture; the `.dmg` is the recommended download.
<!-- endif -->

`SHA256SUMS.txt` lists the SHA-256 checksum of every file. To check a download, put it next to `SHA256SUMS.txt` and compare:

<!-- if windows -->
- Windows (PowerShell): `(Get-FileHash .\LingoPop-<version>-windows-x64-portable.exe -Algorithm SHA256).Hash.ToLower()` must equal the value on that file's line in `SHA256SUMS.txt`.
<!-- endif -->
<!-- if macos -->
- macOS (Terminal, in the download folder): `shasum -a 256 -c SHA256SUMS.txt --ignore-missing` must print `OK` for the file you downloaded.
<!-- endif -->

<!-- if windows -->
### Windows

- This is an unsigned pre-release. Windows SmartScreen may show "Windows protected your PC": click **More info**, then **Run anyway**.
- It needs the Microsoft Edge WebView2 runtime, which is preinstalled on Windows 10 and 11. If the app does not start, install the WebView2 runtime from Microsoft.
- It is portable: there is no installer. Put the exe anywhere you like and run it. Settings, history and the encrypted API keys are stored in `%APPDATA%\LingoPop`.
- LingoPop runs in the notification area (tray); the icon may be in the hidden-icons overflow. Its menu opens Settings, History and About.
- Starting with Windows is a setting: Settings > Main > Startup.
<!-- endif -->

<!-- if macos -->
### macOS

- Requires macOS 13 Ventura or newer.
- Open the `.dmg`, drag **LingoPop Preview** to **Applications**, then eject the disk image. (With the `.zip`: double-click it and move `LingoPop Preview.app` to Applications.) The installed app is `/Applications/LingoPop Preview.app`.
<!-- if macos-adhoc -->
- This build is signed ad hoc, not notarized by Apple, so Gatekeeper blocks the first launch ("Apple could not verify ..."). Click **Done**, open **System Settings > Privacy & Security**, scroll to **Security** and click **Open Anyway** next to the LingoPop Preview message, then confirm. Alternatively, in Terminal: `xattr -dr com.apple.quarantine "/Applications/LingoPop Preview.app"`, then open the app.
<!-- endif -->
- LingoPop Preview is a menu-bar app: it has no Dock icon and no main window. Use its menu-bar icon for Settings, History and Quit.
- Grant the permissions when macOS asks (or from Settings > Main > Permissions):
  - **Accessibility** for the text operations (reading the selected text), Replace and closing the popup with Escape.
  - **Screen Recording** for Screen Translate. After granting it, fully quit LingoPop Preview (menu-bar icon > Quit) and open it again: macOS applies a new Screen Recording grant only after a relaunch.
- macOS may ask once for access to the login keychain: LingoPop Preview keeps the key that encrypts your API keys there. Allow it.
<!-- if macos-adhoc -->
- After an update, macOS may treat the new ad-hoc build as a different app: if a permission is shown as granted but does not work, remove LingoPop Preview from the list in **System Settings > Privacy & Security** (Accessibility or Screen Recording, the **–** button), then add it again or grant it when asked. The keychain prompt may also appear again.
<!-- endif -->
- Settings, history and the encrypted API keys are stored in `~/Library/Application Support/LingoPop-Tauri`.
- LingoPop Preview is a separate app from the earlier Swift LingoPop (different bundle identifier, `app.lingopop.tauri.macos`): both can be installed side by side. It does not import the Swift app's settings, history or API keys; enter your API keys again in Settings > Providers.
<!-- endif -->
