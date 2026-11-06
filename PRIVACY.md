# Privacy Policy

LingoPop is a tray app for Windows and a menu-bar app for macOS. This document describes what data the app handles and where it goes.

## Data the app stores locally

All local data lives in one folder:

- Windows: `%APPDATA%\LingoPop\`
- macOS: `~/Library/Application Support/LingoPop-Tauri/`

In that folder:

- **API keys and provider settings** (base URL, model, referer, region) are stored in `secrets.dat`, encrypted. On Windows the file is protected with Windows DPAPI (current user scope), so only the same Windows user on the same machine can decrypt it. On macOS it is encrypted with AES-256-GCM; the 256-bit key is a random key kept in your login keychain as a non-synchronizable item that stays on this Mac. Both protect the keys at rest; neither is a security boundary against other processes running as your user.
- **Settings** (active provider, target language, hotkeys, history size limit, etc.) are stored in `settings.json` and `ui.json`. They never contain API keys.
- **Translation history** (last N entries) is stored in `history.json`. It includes the original text, the result, and metadata (timestamp, operation, provider, target language).
- **Usage statistics** (token counts per provider and day, never any text) are stored in `usage.json`.
- **A log file** `lingopop.log` with fixed technical messages (errors, permission states). It never contains your text, clipboard contents or API keys.

Nothing is synced: there is no iCloud, no cloud folder and no account. Every file stays on your computer. You can limit or clear history and reset usage statistics in the app, or delete the whole folder at any time.

## Data sent to third parties

When you trigger an operation (translate, fix grammar, rewrite, explain, magic, screen translate), LingoPop sends the selected text and a system prompt to the provider you have configured as active. The endpoint depends on the provider:

- Claude → `api.anthropic.com` (or a custom base URL you configure)
- OpenAI → `api.openai.com` (or a custom base URL, useful with LiteLLM proxies)
- Gemini → `generativelanguage.googleapis.com`
- Grok → `api.x.ai`
- Google (Free) → `translate.googleapis.com/translate_a/single`: an unofficial, undocumented endpoint with no API key; the text is sent in the URL query string, so it can appear in intermediary logs and very long texts may be rejected
- Google Translate → `translation.googleapis.com/language/translate/v2`
- DeepL → `api-free.deepl.com` or `api.deepl.com` (depending on your key type)
- Microsoft Translator → `api.cognitive.microsofttranslator.com`

**Screen Translate:** text is recognized locally on your computer, with the Windows OCR engine on Windows and Apple's Vision framework on macOS. Pressing **Retry** in the Screen Translate popup uploads the captured screen region to the AI provider you choose (Claude, OpenAI, Gemini or Grok) for combined recognition and translation. The image is held in memory only and is never written to disk by LingoPop.

**Fallback:** if the active provider fails with a retry-able error (rate limit, server error, network), LingoPop may resend the same text to the fallback provider you configured for it.

**Update check:** LingoPop asks the GitHub API (`api.github.com`) for the latest release of this repository at startup, every three hours and when you click **Check for updates**. The request carries no personal data besides what any HTTPS request reveals (your IP address and the app version in the User-Agent).

Each provider has its own privacy policy and data-retention practices. LingoPop is not affiliated with any of them; refer to their documentation for how they handle your data.

LingoPop does **not** send any data to the maintainer or to any server other than the provider endpoint you choose and the GitHub update check.

## Clipboard data

When you trigger an operation, LingoPop briefly uses the system clipboard to capture your selected text:

1. The current clipboard contents are saved in memory.
2. A copy keystroke (`Ctrl+C` on Windows, `⌘C` on macOS) is simulated to copy your selection.
3. The selected text is read from the clipboard and sent to your chosen provider.
4. The previous clipboard contents are restored right afterward, unless another app wrote to the clipboard in the meantime (its newer contents are never overwritten).

On Windows, formats that cannot be read immediately (for example delayed-rendering formats) are skipped and not restored; this is logged locally without any clipboard content. On macOS, if the clipboard holds something that cannot be saved and restored exactly (for example a file promise), LingoPop stops the operation and changes nothing.

The temporary clipboard entry that LingoPop writes itself for Replace is marked so they are left out of clipboard history: on Windows it is excluded from Clipboard history (`Win+V`) and the cloud clipboard; on macOS it carries the `org.nspasteboard` transient and concealed markers that clipboard managers honour.

**The clipboard snapshot is never written to disk and is never sent anywhere.** The provider receives only the text needed for the operation: no clipboard metadata, no surrounding context, no other clipboard entries.

If translation history is enabled, the source text and result are saved to `history.json` (local only). You can limit or clear history in Settings.

## Telemetry and analytics

There is no telemetry, no analytics and no crash reporting service. The app makes outbound network requests only to the provider you choose (only when you trigger an operation) and to GitHub for the update check.

## Permissions the app needs

- **Windows:** no special permission. To read your selection LingoPop simulates `Ctrl+C` and reads the clipboard. Windows does not let a normal app send keystrokes to an app running with administrator rights, so selection capture does not work there.
- **macOS:** **Accessibility**, to simulate `⌘C` / `⌘V` for reading the selection and Replace, and to notice Esc for closing the popup; **Screen Recording**, only for Screen Translate. Each one is requested only when an operation needs it.

The app is not sandboxed, so it could read user files in principle, though it does not do so by design.

## Removing your data

Quit LingoPop, delete the app (the portable exe on Windows, `LingoPop Preview.app` on macOS), then delete the data folder listed above. On macOS you can also delete the `app.lingopop.tauri.macos` item in Keychain Access and remove LingoPop Preview from the Accessibility and Screen Recording lists in System Settings.

## Contact

Open an issue in this repository for privacy questions.
