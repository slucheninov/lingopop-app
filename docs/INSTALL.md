# Installing LingoPop

LingoPop is published on the [Releases page](https://github.com/slucheninov/lingopop-app/releases). Everything below the marker comment is also copied into the notes of every release by the release workflow (`scripts/verify-release-assets.py publish`): `<version>` becomes the release version, the Windows or macOS part is left out when the release has no assets for it, and the two ad-hoc-only macOS paragraphs (opening the app past Gatekeeper, and re-granting permissions after an update) are left out when the macOS build is Developer ID signed and notarized. Keep the `<!-- if ... -->` / `<!-- endif -->` markers balanced (the release tests check it).

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

- The exe is not code-signed. Windows SmartScreen may show "Windows protected your PC": click **More info**, then **Run anyway**.
- It needs the Microsoft Edge WebView2 runtime, which is preinstalled on Windows 10 and 11. If the app does not start, install the WebView2 runtime from Microsoft.
- It is portable: there is no installer. Put the exe anywhere you like and run it. Settings, history and the encrypted API keys are stored in `%APPDATA%\LingoPop`.
- LingoPop runs in the notification area (tray); the icon may be in the hidden-icons overflow. Its menu opens Settings, History and About.
- Starting with Windows is a setting: Settings > Main > Startup.
<!-- endif -->

<!-- if macos -->
### macOS

- Requires macOS 13 Ventura or newer.
- Open the `.dmg`, drag **LingoPop** to **Applications**, then eject the disk image. (With the `.zip`: double-click it and move `LingoPop.app` to Applications.) The installed app is `/Applications/LingoPop.app`.
<!-- if macos-adhoc -->
- This build is signed ad hoc, not notarized by Apple, so Gatekeeper blocks the first launch: macOS says it cannot verify the app or check it for malicious software. Close the dialog, then open **System Settings > Privacy & Security** and scroll to **Security**: after one blocked attempt to open the app, an **Open Anyway** button for LingoPop appears there (for about an hour). Click it, confirm with **Open**, and enter your administrator password if macOS asks. On macOS 13 and 14 you can instead Control-click the app in Applications, choose **Open** and confirm. Alternatively, in Terminal: `xattr -dr com.apple.quarantine "/Applications/LingoPop.app"`, then open the app.
<!-- endif -->
- LingoPop is a menu-bar app: it has no Dock icon and no main window. Use its menu-bar icon for Settings, History and Quit.
- Grant the permissions when macOS asks (or from Settings > Main > Permissions):
  - **Accessibility** for the text operations (reading the selected text), Replace and closing the popup with Escape.
  - **Screen Recording** for Screen Translate. After granting it, fully quit LingoPop (menu-bar icon > Quit) and open it again: macOS applies a new Screen Recording grant only after a relaunch.
- macOS may ask once for access to the login keychain: LingoPop keeps the key that encrypts your API keys there. Allow it.
<!-- if macos-adhoc -->
- After an update, macOS may treat the new ad-hoc build as a different app: if a permission is shown as granted but does not work, remove LingoPop from the list in **System Settings > Privacy & Security** (Accessibility or Screen Recording, the **–** button), then add it again or grant it when asked. The keychain prompt may also appear again.
<!-- endif -->
- Settings, history and the encrypted API keys are stored in `~/Library/Application Support/LingoPop-Tauri`.
- This LingoPop is a separate app from the earlier Swift LingoPop (different bundle identifier, `app.lingopop.tauri.macos`, and data folder). Both are named `LingoPop.app`, so installing this one into Applications replaces the Swift app there (Finder asks first); quit the Swift app before that. It does not import the Swift app's settings, history or API keys; enter your API keys again in Settings > Providers.
<!-- endif -->
