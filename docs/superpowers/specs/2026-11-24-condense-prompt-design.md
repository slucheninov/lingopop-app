# Condense prompt operation: design

Date: 2026-11-24. Status: draft for approval.

## Intent

Selected text (a task draft, a stream of thoughts, a chat fragment) is turned by the AI into the
shortest clear prompt another AI can act on. The point is to send less context to Claude, ChatGPT
or an agent without losing meaning. It is a separate operation next to Translate and Magic.

## Goals

- New operation, hotkey Option+P on macOS, from the tray menu too.
- Input up to 5000 characters; output is a minimal, unambiguous prompt.
- The result is written in the target language from Settings (like Explain; no auto-detect, no
  language pairs).
- Popup like the other text operations: Copy, Replace, Rewrite, Explain, Retry, retranslate menu.

## Non-goals

- No editable prompt in Settings (Magic already covers custom prompts; can be added later).
- No separate hard 5000 limit: the existing global `maxInputChars` (default 5000) applies.
- No new provider support: translation-only providers (Google Free, Google Translate, DeepL,
  Microsoft) do not support it, exactly as for Rewrite / Explain today.

## Anti-goals

- The condensed prompt must not drop requirements, constraints, names, numbers, code, paths or
  identifiers. Shorter but wrong is a failure.
- It must not answer or execute the request; it only rewrites it as a prompt.

## Approach (chosen: A)

- **A. New first-class `AIOperation` with a fixed prompt** - chosen: smallest UI, predictable output.
- B. Same with an editable prompt in Settings - rejected: doubles the UI, Magic already does this.
- C. Dedicated hard 5000 limit - rejected: the global limit is enough.

## Design

- Core `AIOperation::CondensePrompt`, raw value `"condensePrompt"` (new frozen value), appended
  last to `ALL` (after Magic). English name `Condense prompt`, short name (History) `Prompt`.
  Unknown-value handling already keeps older readers safe (history/usage keep raw strings).
- `prompts::system_prompt` gets a fixed text in the style of the others, with `{FENCE}` and
  `NO_EM_DASH`, roughly: "You are a prompt engineer. Your only job is to condense text into a
  prompt for an AI assistant. ... Rewrite the content inside <text> tags as the shortest clear,
  unambiguous prompt in {target_language}. Keep every requirement, constraint, name, number, code
  fragment, path and identifier. Remove filler, repetition and pleasantries. Do not answer or
  carry out the request. Output only the prompt." Output is sanitized like the other text ops.
- Provider capabilities: LLM providers support it via `AIOperation::ALL`; translation providers
  keep `TRANSLATE_OPERATIONS`, so the runner's existing unsupported-operation path applies.
- Target language: `target_language_for` already returns plain `target_language` for any
  non-Translate operation; nothing to change.
- Hotkey: default Option+P (`KeyP` + Alt) on macOS (`HostKind::Unix`) for fresh settings; no
  default on Windows (Ctrl+Alt+A is AltGr+A = Polish "ą", and defaults avoid AltGr letters).
- Shell / UI: tray `op:condensePrompt` item, Shortcuts row, loading label, operation names in
  `types.ts` / `settingsTypes.ts`, History label, Usage needs nothing new. New strings go into the
  shared catalogs (pl / uk / de) in the same commit.
- Tests: raw value frozen list, prompt literal anchor + fence, capabilities table, default hotkey
  per host, UI rows / catalog coverage.

## Calls made by Claude (check these)

- Names: raw `condensePrompt`, UI `Condense prompt`, History `Prompt`, Shortcuts icon `⇲`.
- Existing installs: a saved `settings.json` without the new hotkey stays without it (absence
  cannot be told apart from "cleared"); the user sets Option+P once in Shortcuts. Fresh installs
  get it by default.
- Option+P takes the "π" character on macOS while LingoPop runs (same trade-off as Option+T).
- Option+P instead of the first idea Option+A: Option+A types "ą" on the Polish Pro layout.
  No default chord with Option / AltGr may use A C E L N O S X Z (test
  `every_default_hotkey_avoids_polish_option_letters`).

## Left to the builder

- Exact prompt wording within the rules above; translations of the new UI strings.
