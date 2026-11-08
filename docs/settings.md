# LingoPop settings

Open Settings from the tray icon (Windows) or the menu-bar icon (macOS): **Settings…**. On Windows, starting LingoPop a second time also opens Settings. Every change is saved immediately, except the provider form and the Magic prompt, which have a **Save** button.

- [Main](#main)
- [Providers](#providers)
- [Shortcuts](#shortcuts)
- [Usage](#usage)
- [Magic](#magic)
- [About](#about)
- [Tray and menu-bar menu](#tray-and-menu-bar-menu)
- [The popup](#the-popup)
- [History](#history)
- [Where LingoPop keeps its data](#where-lingopop-keeps-its-data)

## Main

- **Permissions** (macOS only): the state of **Accessibility** and **Screen Recording**, with **Request**, **Open Settings** and **Re-check**. After granting Screen Recording, LingoPop shows a **Relaunch** button: macOS applies that permission only after a restart. See [Grant Accessibility](../README.md#grant-accessibility-and-screen-recording).
- **Target language**: the language results are written in. Turn on **Auto-detect input language** to use **language pairs** (source → target): the source language of the selected text is detected on your computer and the matching pair chooses the target; any other source goes to the **Default target**. Pairs apply to Translate and to the text Screen Translate recognized; Fix grammar, Rewrite, Explain and Magic always use the Default target. The defaults are Russian → Ukrainian, Ukrainian → Russian and English → Ukrainian.
- **Popup**: **Result font size**, 11 to 20 pt.
- **Language**: the interface language (System, English, Polski, Українська, Deutsch). It applies after a restart; LingoPop offers a **Relaunch** button.
- **Limits**:
  - **Max input characters**: longer selections are refused before anything is sent (default 5000, 0 = no limit).
  - **History size**: how many results are kept in History (0 to 100, default 100).
  - **Display**: how many recent results the tray / menu-bar menu shows (0 to 100, default 10).
- **Clipboard History**: **Clipboard history size** (0–1000, default 100; 0 disables recording and clears stored entries) and **Clipboard entries in tray** (0–50, default 10). Entries up to 64 KiB are kept locally, subject to an 8 MiB total text limit.
- **Startup**: **Launch at login** and **Auto-copy OCR/translation results to clipboard** (Screen Translate copies the recognized text right away).

## Providers

The sidebar lists all eight providers. The star marks the **active** provider, the one every hotkey and menu operation uses. Under each name you see whether a key is set, and its fallback.

For the selected provider:

- **Set as active**: switch the provider your next operation uses.
- **API key**: paste the key and click **Save**. A saved key is never shown again; **Enter a new key to replace** it, or **Clear** it. Google (Free) needs no key. How to get a key for each provider: [api-keys.md](api-keys.md).
- **Endpoint** / **Base URL** and **Model**: the defaults work out of the box. Change them only for a proxy or a different model; **Reset to default endpoint** puts the default back.
- **HTTP Referer (optional)**, Google Translate only: for keys restricted by HTTP referrer in the Google Cloud console.
- **Region (optional)**, Microsoft Translator only: needed for multi-service or regional Azure resources; leave it empty for a global free-tier (F0) resource.
- **DeepL**: leave the base URL empty. Keys ending in `:fx` use the free endpoint, all others the Pro endpoint.
- **Fallback**: another provider that is tried once when this one fails with a rate limit (429), a server error (5xx) or a network error.
- **How to get an API key** opens the matching section of [api-keys.md](api-keys.md).

Fix grammar, Rewrite, Explain and Magic need an AI provider: Claude, OpenAI, Gemini or Grok. Google (Free), Google Translate, DeepL and Microsoft Translator translate only.

If the encrypted key file cannot be decrypted (for example after it was copied from another computer), LingoPop moves it aside as `secrets.dat.bak-<time>` and starts with no keys: enter them again. On macOS, when the keychain item that holds the encryption key is missing or cannot be read, the file is kept locked instead and the form offers **Reset stored secrets**: it clears all stored keys and provider settings so you can enter them again.

## Shortcuts

One global shortcut per operation: **Translate**, **Fix grammar**, **Rewrite**, **Explain**, **Screen Translate** and **Magic**. Only Translate has a default: **Ctrl+Alt+T** on Windows, **⌥T** (Option+T) on macOS.

Click a field and press the combination: at least one modifier (Windows: Ctrl, Alt, Shift, Win; macOS: ⌃ ⌥ ⇧ ⌘) plus a key. **Esc** cancels, the clear button removes the shortcut. LingoPop tells you when a combination is invalid, already used by another operation, or taken by another app.

On macOS, a shortcut that the earlier Swift LingoPop also uses is paused while that app is running; the tab shows which ones.

## Usage

Tokens and estimated cost of the AI providers (Claude, OpenAI, Gemini, Grok) for **1 day**, **7 days** or **30 days**: three summary cards, a chart and a table per provider. Costs are estimates from the providers' public prices; a model without a known price shows no cost. Translation-only providers report no tokens and are not counted. **Reset stats…** deletes the usage history. The statistics contain no text.

## Magic

The system prompt of the Magic operation (up to 2000 characters). `{{targetLanguage}}` is replaced with your target language when the prompt is sent. **Save** applies it, **Reset to default** restores the built-in prompt (fix grammar, improve style and translate in one call).

## About

The version, **Check for updates** (LingoPop also checks GitHub releases every three hours) and the privacy policy. When a new stable version is out a banner offers it:

- **Update** downloads the new version with a progress bar, checks its signature and installs it. **Cancel** stops a download.
- **Restart** starts the new version. Without it the new version starts the next time you open LingoPop.
- On a failure the tab shows the reason and a **Download** link to the Releases page for a manual install.
- Where the install can't be replaced (development build, folder not writable, app running from the disk image, ad-hoc signed macOS build) only the **Download** link is shown.

Updates are signed; on macOS the new app must carry the same release certificate, so the Accessibility and Screen Recording grants are kept. The first release with this is 0.2.1: install it by hand once, and re-grant the macOS permissions one last time.

## Tray and menu-bar menu

- The latest results, newest first. A click copies the original text back to the clipboard, so the menu also works as a clipboard history. The first ten have quick keys (Windows `1`…`0`, macOS `⌘1`…`⌘0`).
- The six operations with their shortcuts. An operation started from the menu works on the text you had selected in the window you used last.
- **Target**: switch the target language.
- **Clipboard History** lists recent copied text; click an entry to copy it again. **Clipboard History…** opens all retained entries, with controls to delete an entry or clear the list.
- **History…**, **Settings…**, **About LingoPop** and **Quit LingoPop**.

The icon animates while an operation runs and turns red for three seconds after an error.

## The popup

The popup opens next to the cursor without taking the focus from your app.

- **Copy** the result, or **Replace** the selection with it (the original line breaks around the selection are kept).
- **Rewrite** or **Explain** the result, choose another target language from the language menu, or **Retry** with another provider.
- When the fallback provider answered, the header says so.
- Screen Translate shows the recognized text first: **Translate** it with the active provider (or another one from the menu), or **Retry** the screenshot with an AI provider that reads images itself.
- Close it with **Esc** or by clicking outside it. On macOS, Esc needs the Accessibility permission.

## History

**History…** in the menu opens all saved results (up to the History size): search, copy the original or the result, delete one entry or **Clear** everything.

## Where LingoPop keeps its data

| Data | Windows | macOS |
|---|---|---|
| Folder | `%APPDATA%\LingoPop` | `~/Library/Application Support/LingoPop-Tauri` |
| Settings | `settings.json`, `ui.json` | `settings.json`, `ui.json` |
| History | `history.json` | `history.json` |
| Clipboard history | `clipboard-history.json` | `clipboard-history.json` |
| Usage statistics | `usage.json` | `usage.json` |
| API keys and provider settings | `secrets.dat` (Windows DPAPI) | `secrets.dat` (AES-256-GCM, key in the login keychain) |
| Log (no text, no keys) | `lingopop.log` | `lingopop.log` |

Quit LingoPop first (otherwise it may write its settings and history back), then delete the folder to reset LingoPop completely.
