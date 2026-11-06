<p align="center">
  <img src="docs/icon.png" alt="LingoPop" width="128" height="128"/>
</p>

<h1 align="center">LingoPop</h1>

<p align="center">
  Select text anywhere, press a hotkey, and get an AI translation, grammar fix, rewrite or explanation in a popup next to your cursor.<br/>
  A tray app for <b>Windows</b> and a menu-bar app for <b>macOS</b>.
</p>

<p align="center">
  <a href="https://github.com/slucheninov/lingopop-app/releases/latest"><img src="https://img.shields.io/github/v/release/slucheninov/lingopop-app?label=Download&style=for-the-badge&color=blue" alt="Latest release"/></a>
  <a href="https://github.com/slucheninov/lingopop-app/releases"><img src="https://img.shields.io/github/downloads/slucheninov/lingopop-app/total?style=for-the-badge&color=blue" alt="Downloads"/></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Windows 10 | 11"/>
  <img src="https://img.shields.io/badge/macOS-13%2B-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS 13+"/>
  <img src="https://img.shields.io/badge/arch-x64%20%7C%20Apple%20Silicon%20%7C%20Intel-lightgrey?style=for-the-badge" alt="Windows x64, macOS Apple Silicon and Intel"/>
</p>

<p align="center">
  <a href="SECURITY.md"><img src="https://img.shields.io/badge/API%20keys-encrypted%20locally-success?style=for-the-badge&logo=letsencrypt&logoColor=white" alt="API keys encrypted locally"/></a>
  <a href="PRIVACY.md"><img src="https://img.shields.io/badge/telemetry-none-success?style=for-the-badge" alt="No telemetry"/></a>
  <a href="https://github.com/slucheninov/lingopop-app/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/slucheninov/lingopop-app/ci.yml?branch=main&label=CI&style=for-the-badge" alt="CI"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/slucheninov/lingopop-app?style=for-the-badge" alt="MIT License"/></a>
</p>

<!--
## Screenshots

Planned files in docs/screenshots/: popup.png, popup-screen.png, menu-windows.png, menu-macos.png,
settings-main.png, settings-providers.png, settings-shortcuts.png, settings-magic.png, history.png.
-->

## Features

- 🌐 **Translate** into 28 languages. Optional auto-detect picks the target from your language pairs (defaults: Russian ↔ Ukrainian, English → Ukrainian); everything else goes to the default target.
- ✓ **Fix grammar**: spelling, punctuation and grammar only; meaning and style stay the same.
- ✨ **Rewrite**: clearer wording in the same language, meaning and tone preserved.
- 💡 **Explain**: a short explanation of the selected word, phrase or text.
- 🖼️ **Screen Translate**: press the hotkey, drag a frame over any part of the screen and get the recognized text at once (on-device OCR: Windows OCR or Apple Vision). Press **Translate** in the popup to translate it, or just copy the text. Works across several monitors.
- 🪄 **Magic**: one hotkey that fixes grammar, improves style and translates in a single AI call. Write your own prompt in **Settings → Magic**; `{{targetLanguage}}` is replaced with your target language.
- ⌨️ A global hotkey for each of the six operations. **Translate** ships with **Ctrl+Alt+T** on Windows and **⌥T** on macOS; the others are yours to assign.
- 📋 Popup actions: **Copy**, **Replace** (pastes the result back into the app you came from), **Rewrite**, **Explain**, **Retry with another provider** and re-translate into another language.
- 🤖 Eight providers: **Claude**, **OpenAI**, **Gemini**, **Grok** (all operations, plus image recognition for Screen Translate), **Google (Free)**, **Google Translate**, **DeepL** and **Microsoft Translator** (translation).
- 🆓 Works out of the box: the default provider, Google (Free), needs no API key.
- 🔁 Per-provider fallback: on a rate limit, a server error or a network error, LingoPop retries once with the fallback you chose.
- 📜 History of up to 100 results with search. The latest ones are in the tray / menu-bar menu, one click copies them back (the first ten have quick keys).
- 📊 Usage statistics: tokens and estimated cost per provider for 1, 7 and 30 days.
- 🌍 Interface in English, Polish, Ukrainian and German.
- 🔒 API keys are encrypted on your computer, nothing is synced, no telemetry. See [Privacy and security](#privacy-and-security).

## Download

Get the files from the [latest release](https://github.com/slucheninov/lingopop-app/releases/latest):

| Your computer | File |
|---|---|
| Windows 10 / 11 (x64) | `LingoPop-<version>-windows-x64-portable.exe` |
| Mac with Apple Silicon (M1 and later), macOS 13+ | `LingoPop-<version>-macos-arm64.dmg` |
| Mac with an Intel processor, macOS 13+ | `LingoPop-<version>-macos-x64.dmg` |

Not sure which Mac you have? Apple menu → **About This Mac**: "Chip: Apple M…" means Apple Silicon, "Processor: Intel" means Intel. The `.zip` files contain the same app as the `.dmg`. `SHA256SUMS.txt` lists the checksum of every file ([how to check a download](docs/INSTALL.md#which-file-to-download)).

## Install on Windows

1. Download `LingoPop-<version>-windows-x64-portable.exe`. It is a portable app: there is no installer, put the exe wherever you like (for example in `Documents` or `C:\Tools`).
2. Run it. The exe is not code-signed, so Windows SmartScreen may show **"Windows protected your PC"**: click **More info**, then **Run anyway**. You only see this once.
3. LingoPop starts in the notification area (tray), next to the clock. If you don't see the icon, open the hidden icons with the **^** arrow; you can drag it onto the taskbar to keep it visible.
4. Select some text in any app and press **Ctrl+Alt+T**.

Good to know:

- LingoPop needs the Microsoft Edge WebView2 runtime, which is preinstalled on Windows 10 and 11. If the app doesn't start, install the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) from Microsoft.
- To start LingoPop with Windows, turn on **Settings → Main → Startup → Launch at login**.
- Windows does not let a normal app read the selection of an app running as administrator. In such windows LingoPop shows a message instead of a translation.
- To update, replace the exe with the new one. Your settings stay in `%APPDATA%\LingoPop`.

## Install on macOS

1. Download the `.dmg` for your Mac and open it.
2. Drag **LingoPop** into **Applications**, then eject the disk image.
3. **First launch.** The app is not notarized by Apple, so the first time macOS refuses to open it ("cannot verify that it is free of malware"). Close the dialog, then:
   - open **System Settings → Privacy & Security**, scroll down to **Security** and click **Open Anyway** next to LingoPop (the button appears after one blocked attempt and stays for about an hour), confirm with **Open** and enter your password if asked;
   - or, on macOS 13 and 14: Control-click the app in **Applications** → **Open** → **Open**;
   - or, in Terminal: `xattr -dr com.apple.quarantine "/Applications/LingoPop.app"`, then open the app normally.
4. LingoPop lives in the menu bar: it has no Dock icon and no main window. Click its icon for Settings, History and Quit.
5. Grant the permissions (next section).

When macOS asks whether LingoPop may use the login keychain, click **Always Allow**: the key that encrypts your API keys is stored there.

### Grant Accessibility (and Screen Recording)

macOS asks for two permissions. LingoPop asks for each one only when an operation needs it, and **Settings → Main → Permissions** shows their status with **Request** and **Open Settings** buttons.

| Permission | Needed for |
|---|---|
| **Accessibility** | Reading the selected text (Translate, Fix grammar, Rewrite, Explain, Magic), **Replace**, and closing the popup with **Esc** |
| **Screen Recording** | **Screen Translate** only |

**Accessibility**

1. Select some text and press **⌥T** (or click **Request** in **Settings → Main → Permissions**). macOS shows "LingoPop would like to control this computer using accessibility features": click **Open System Settings**.
2. In **System Settings → Privacy & Security → Accessibility**, turn on the switch next to **LingoPop**. If it isn't in the list, click **+**, choose `/Applications/LingoPop.app` and turn it on.
3. Go back to LingoPop and try again. If it still says the permission is missing, quit LingoPop (menu-bar icon → **Quit LingoPop**) and open it again.

**Screen Recording**

1. Start Screen Translate (or click **Request** next to Screen Recording). macOS shows the Screen Recording prompt: click **Open System Settings**.
2. In **System Settings → Privacy & Security → Screen & System Audio Recording** (**Screen Recording** on macOS 13 and 14), turn on **LingoPop**.
3. **Fully quit LingoPop and open it again**: macOS applies a new Screen Recording permission only after a relaunch. Settings shows a **Relaunch** button for this.

> [!NOTE]
> **After an update** macOS may treat the new build as a different app: the switch is still on, but the permission doesn't work. Select **LingoPop** in the Accessibility (or Screen Recording) list, remove it with **−**, then add it again with **+** or grant it when asked. The keychain question may also come back once.

Good to know:

- This LingoPop is a separate app from the earlier Swift [LingoPop](https://github.com/slucheninov/lingopop) (a different bundle identifier and data folder). Both are named `LingoPop.app`, so dragging this one into Applications replaces the Swift app there (Finder asks first); quit the Swift app before that. It does not import that app's settings, history or API keys.
- To start LingoPop at login, turn on **Settings → Main → Startup → Launch at login**.
- To update, replace the app in **Applications** with the new one (quit LingoPop first). Your settings stay in `~/Library/Application Support/LingoPop-Tauri`.

## First steps

1. Select text in any app (a browser, a mail, a chat, a document).
2. Press the Translate hotkey (**Ctrl+Alt+T** on Windows, **⌥T** on macOS), or pick an operation in the tray / menu-bar menu.
3. The popup appears next to the cursor. **Copy** the result, **Replace** the selection with it, or try another provider or language.
4. Close the popup with **Esc** or by clicking anywhere outside it.

Then open **Settings** from the tray / menu-bar icon:

- **Main**: target language and language pairs, popup font size, app language, limits, launch at login.
- **Providers**: pick the active provider, enter API keys, set a fallback.
- **Shortcuts**: assign hotkeys to Fix grammar, Rewrite, Explain, Screen Translate and Magic.

The full tour of every tab is in [docs/settings.md](docs/settings.md).

## Providers and API keys

Google (Free) works without a key. For the others, enter a key in **Settings → Providers**; step-by-step instructions for every provider are in [docs/api-keys.md](docs/api-keys.md).

| Provider | Operations | Get a key |
|---|---|---|
| Claude | All operations, Screen Translate image recognition | [console.anthropic.com](https://console.anthropic.com/) |
| OpenAI | All operations, Screen Translate image recognition | [platform.openai.com/api-keys](https://platform.openai.com/api-keys) |
| Gemini | All operations, Screen Translate image recognition | [aistudio.google.com/apikey](https://aistudio.google.com/apikey) |
| Grok | All operations, Screen Translate image recognition | [console.x.ai](https://console.x.ai/) |
| Google (Free) | Translate | No key: unofficial public endpoint |
| Google Translate | Translate | [console.cloud.google.com](https://console.cloud.google.com/apis/credentials) (enable the Cloud Translation API) |
| DeepL | Translate | [deepl.com/pro-api](https://www.deepl.com/pro-api), free plan available |
| Microsoft Translator | Translate | [Azure portal](https://portal.azure.com/), free tier F0 gives 2M characters a month |

Fix grammar, Rewrite, Explain and Magic need Claude, OpenAI, Gemini or Grok.

## Privacy and security

- **API keys stay on your computer, encrypted.** Windows: `%APPDATA%\LingoPop\secrets.dat`, protected with Windows DPAPI for your user account. macOS: `~/Library/Application Support/LingoPop-Tauri/secrets.dat`, encrypted with AES-256-GCM; the encryption key is in your login keychain and never leaves this Mac. Keys are never shown again after saving and never written to logs.
- **Your text goes only to the provider you chose**, and only when you start an operation. No telemetry, no analytics, no accounts, no sync.
- **Screen Translate recognizes text on your computer.** A screenshot is sent to a provider only when you press **Retry** with one of the AI providers.
- **Your clipboard is restored.** LingoPop copies the selection through the clipboard and puts your previous contents back right after. On Windows, clipboard formats that can't be read at once (rare, delayed-rendering ones) are not restored; on macOS, if the clipboard holds something that can't be restored exactly, LingoPop stops and changes nothing. The temporary entry used by **Replace** is marked so clipboard history tools skip it; the selection itself is copied by your app, so a clipboard history tool may keep it. Screen Translate copies the recognized text to the clipboard by default, replacing what was there (turn off **Settings → Main → Auto-copy OCR/translation results to clipboard**). Details on what else lands on the clipboard: [PRIVACY.md](PRIVACY.md#clipboard-data).
- History and usage statistics are plain local files that you can clear in the app.

Details: [PRIVACY.md](PRIVACY.md). Found a vulnerability? Please report it privately as described in [SECURITY.md](SECURITY.md).

## FAQ

**Why does Windows or macOS warn me about the app?**
The builds are not signed with a paid certificate (Microsoft code signing, Apple Developer ID), so the system cannot verify the publisher and asks you to confirm the first launch. Every release lists SHA-256 checksums in `SHA256SUMS.txt`, so you can check that your download is the published file.

**Nothing happens when I press the hotkey.**
Make sure some text is selected and the tray / menu-bar icon is there. Another app may already use the same shortcut: **Settings → Shortcuts** shows a conflict, pick another combination. On macOS check the Accessibility permission (see above).

**Replace pasted nothing.**
If the original app is no longer in front, LingoPop leaves the result on the clipboard and tells you to paste it yourself (Ctrl+V / ⌘V).

**How do I check for updates?**
**Settings → About → Check for updates**. LingoPop also checks GitHub releases in the background every three hours and shows a banner when a new version is out.

**How do I uninstall?**
Quit LingoPop and delete the exe (Windows) or `LingoPop.app` (macOS). To remove your settings, history and keys too, delete `%APPDATA%\LingoPop` or `~/Library/Application Support/LingoPop-Tauri`. On macOS you can also remove LingoPop from the Accessibility and Screen Recording lists.

## Feedback

Found a bug or have an idea? Open an [issue](https://github.com/slucheninov/lingopop-app/issues/new/choose) with your OS version, the LingoPop version (**Settings → About**) and the steps to reproduce. Never include API keys.

- What changed in each version: [CHANGELOG.md](CHANGELOG.md)
- Want to contribute: [CONTRIBUTING.md](CONTRIBUTING.md)

## License

LingoPop is released under the [MIT License](LICENSE).
