# Privacy Policy

LingoPop is a Windows tray app. This document describes what data the app handles and where it goes.

## Data the app stores locally

All local data lives in `%APPDATA%\LingoPop\`:

- **API keys and provider settings** (base URL, model, referer, region) for AI providers are stored in `secrets.dat`. The file is protected with Windows DPAPI (current user scope), so it can only be decrypted by the same Windows user on the same machine. This protects the keys at rest, but it is not a security boundary against other processes running as your user. DeepL and Microsoft Translator keys are stored the same way.
- **Settings** (preferred provider, target language, hotkeys, history size limit, etc.) are stored in `settings.json`. They never contain API keys.
- **Translation history** (last N entries) is stored in `history.json`. It includes the original text, the translated text, and metadata (timestamp, provider, target language).
- **Usage statistics** (token counts and estimated cost per provider and day, never any text) are stored in `usage.json`.

Nothing is synced: there is no iCloud, no cloud folder and no account. Every file stays on your PC. You can limit or clear history in Settings, or delete the whole folder at any time.

## Data sent to third parties

When you trigger an operation (translate, fix grammar, rewrite, explain, magic, screen translate), LingoPop sends the selected text and a system prompt to the AI provider you have configured as active. The endpoint depends on the provider:

- Claude → `api.anthropic.com` (or a custom base URL you configure)
- OpenAI → `api.openai.com` (or a custom base URL — useful with LiteLLM proxies)
- Gemini → `generativelanguage.googleapis.com`
- Grok → `api.x.ai`
- Google (Free) → `translate.googleapis.com/translate_a/single` — an unofficial, undocumented endpoint with no API key; the text is sent in the URL query string, so it can appear in intermediary logs and very long texts may be rejected
- Google Translate → `translation.googleapis.com/language/translate/v2`
- DeepL → `api-free.deepl.com` or `api.deepl.com` (depending on your key type)
- Microsoft Translator → `api.cognitive.microsofttranslator.com`

**Screen Translate:** text is recognized locally on your PC with the Windows OCR engine. Pressing **Retry** in the Screen Translate popup uploads the captured screenshot region to the AI provider you choose (Claude, OpenAI, Gemini or Grok) for combined OCR and translation. The image is held in memory only and is not written to disk by LingoPop.

**Fallback:** if the active provider fails with a retry-able error (rate limit, server error, network), LingoPop may resend the same text to the fallback provider you configured for it.

Each provider has its own privacy policy and data-retention practices. LingoPop is not affiliated with any of them; refer to their documentation for how they handle your data.

LingoPop does **not** send any data to the maintainer or to any server other than the AI provider endpoint you choose.

## Clipboard data

When you trigger an operation, LingoPop temporarily reads the system clipboard to capture your selected text. This works as follows:

1. The current clipboard contents (the formats that can be read immediately) are saved in memory.
2. A `Ctrl+C` keystroke is simulated to copy your selection.
3. The selected text is read from the clipboard and sent to your chosen AI provider to perform the operation.
4. The original clipboard contents are restored immediately afterward.

Formats that cannot be read immediately (for example delayed-rendering formats) are skipped and not restored; this is logged locally without any clipboard content.

The temporary clipboard entries that LingoPop writes itself (when capturing the selection, and for Replace) are marked so that Windows excludes them from Clipboard history (`Win+V`) and from the cloud clipboard.

**The clipboard snapshot is never written to disk and is never sent anywhere except the AI provider you have chosen.** The AI provider receives only the text needed for the operation — no clipboard metadata, no surrounding context, no other clipboard entries.

If translation history is enabled, the source text and result are saved to `%APPDATA%\LingoPop\history.json` (local only). You can limit or clear history in Settings.

### Privacy summary for clipboard data

- Clipboard data is only read in memory for the duration of the operation.
- No clipboard data is stored by LingoPop beyond the optional translation history entries.
- No clipboard data is transmitted to LingoPop or any third party other than the AI provider you select.
- No analytics are collected on your clipboard content.
- Your clipboard history (outside of LingoPop's own translation history) remains entirely private and under your control.

## Telemetry and analytics

There is no telemetry, no analytics, no crash reporting service. The app makes outbound network requests only to the AI provider you choose, and only when you trigger an operation.

## Permissions the app needs

LingoPop does not ask for any special Windows permission. To read your selection it simulates `Ctrl+C` and reads the clipboard, then restores the previous clipboard contents. Windows does not let a normal app send keystrokes to an app running with administrator rights, so selection capture does not work there. The app is not sandboxed, so it can read user files in principle, though it does not do so by design.

## Where your data goes when you uninstall

Uninstall LingoPop from Windows Settings, then optionally delete `%APPDATA%\LingoPop\`.

## Contact

Open an issue in this repository for privacy questions.
