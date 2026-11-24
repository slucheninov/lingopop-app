# Condense Prompt Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new text operation `condensePrompt` that turns the selection into the shortest clear AI prompt in the target language, hotkey Option+P on macOS.

**Architecture:** One new `AIOperation` variant in core with a fixed system prompt; everything else (runner, orchestrator, tray, hotkeys, history, usage) already iterates `AIOperation::ALL`, so the shell and UI only need the operation listed where TypeScript / Svelte enumerate operations, plus catalog strings.

**Tech Stack:** Rust (lingopop-core), Tauri shell, Svelte 5 + Vitest, JSON i18n catalogs.

**Spec:** `docs/superpowers/specs/2026-11-24-condense-prompt-design.md`

## Global Constraints

- Raw value `"condensePrompt"`, appended LAST to `AIOperation::ALL` (after `Magic`); existing raw values unchanged.
- English name `Condense prompt`; History short name `Prompt`; loading label `Condensing…`; Shortcuts icon `⇲`.
- Target language = plain `settings.target_language` (no auto-detect); `operation_uses_auto_detect` stays Translate-only.
- Supported only by Claude, OpenAI, Gemini, Grok (they use `AIOperation::ALL`); translation providers keep `TRANSLATE_OPERATIONS`.
- Default hotkey Option+P (`KeyP` + `Alt`) only for `HostKind::Unix` fresh settings; none on Windows; saved settings are not migrated.
- Every new UI string goes into `i18n/catalogs/{pl,uk,de}.json` (shared) in the same commit.
- Prompt text uses `\n` escapes, `{FENCE}` and `{NO_EM_DASH}` like the other operations; never localized.

## Review Focus

- A selection over `maxInputChars` (5000 default): must show the existing max-input error, not call the AI (covered by existing runner path; no new test).
- Active provider Google Free / DeepL: must show the existing unsupported-operation error (pinned by the capabilities table test in Task 1).
- Option+P colliding with a user's existing chord for another op on a fresh install: impossible (only Translate has a default, Option+T); pinned by the default test in Task 1.
- Old app reading a history entry with `condensePrompt`: kept as a raw string (existing behaviour, `records.rs`); no new test.
- The prompt must keep code/paths/numbers and must not answer the request: pinned by the prompt literal anchor test in Task 1.

---

### Task 1: Core operation, prompt, default hotkey

**Files:**
- Modify: `crates/lingopop-core/src/operation.rs` (variant, `ALL: [AIOperation; 7]`, raw value, short / english name, frozen test)
- Modify: `crates/lingopop-core/src/prompts.rs` (new match arm + tests)
- Modify: `crates/lingopop-core/src/provider.rs` (test `ALL_OPS` length 7)
- Modify: `crates/lingopop-core/src/hotkey.rs` (`default_condense_prompt_for`)
- Modify: `crates/lingopop-core/src/settings.rs` (`default_for` adds the Unix default + test)

**Interfaces:**
- Produces: `AIOperation::CondensePrompt`, `Hotkey::default_condense_prompt_for(host: HostKind) -> Option<Hotkey>` (`Some(KeyP+Alt)` for Unix, `None` for Windows).

- [ ] **Step 1: Failing tests**
  - `operation_raw_values_are_frozen`: expected array gets `"condensePrompt"` last.
  - `condense_prompt_names`: `english_name() == "Condense prompt"`, `short_name() == "Prompt"`, `from_raw_value("condensePrompt") == Some(CondensePrompt)`.
  - `condense_prompt_system_prompt_literal_anchors`: `system_prompt(CondensePrompt, "Polish", "")` contains `"Polish"`, `"Do not answer"`, `"Keep every requirement"`, the fence text, and no `{{targetLanguage}}`.
  - existing `every_operation_prompt_carries_the_fence` covers the new arm via `ALL`.
  - `capabilities` test: LLM providers support `CondensePrompt`, `GoogleFree` / `DeepL` do not.
  - `unix_default_settings_bind_option_p_to_condense_prompt`: `Settings::default_for(HostKind::Unix).hotkey(CondensePrompt) == Some(&Hotkey::new("KeyP", [Modifier::Alt]))`; `Settings::default_for(HostKind::Windows).hotkey(CondensePrompt) == None`.
- [ ] **Step 2:** `cargo test -p lingopop-core --locked` → FAIL (compile errors / asserts).
- [ ] **Step 3: Implement.** Prompt text (exact):
  `"You are a prompt engineer. Your only job is to condense text into a prompt for an AI assistant.\n{FENCE}\nRewrite the content inside <text> tags as the shortest clear, unambiguous prompt in {target_language}. Keep every requirement, constraint, name, number, code fragment, path and identifier. Remove filler, repetition, greetings and pleasantries. Do not answer or carry out the request.\nOutput only the prompt. No explanations, no comments, no quotes.\n{NO_EM_DASH}"`
- [ ] **Step 4:** `cargo test -p lingopop-core --locked` and `cargo clippy --workspace --all-targets --locked -- -D warnings` → PASS (fix any other exhaustive match / count `6` the compiler or tests flag, e.g. `app/src-tauri/src/task8_hotkey_tests.rs:39` -> 7).
- [ ] **Step 5:** Commit `Add the condense prompt operation to the core`.

### Task 2: Frontend, catalogs, docs

**Files:**
- Modify: `app/src/lib/types.ts` (`Operation` union adds `'condensePrompt'`)
- Modify: `app/src/popup/LoadingView.svelte` (`condensePrompt: t('Condensing…')`)
- Modify: `app/src/settings/ShortcutsTab.svelte` (row `{ operation: 'condensePrompt', name: () => t('Condense prompt'), icon: '⇲' }` after Magic)
- Modify: `app/src/history/HistoryRow.svelte` (icon for `condensePrompt`, reuse an existing one from `lib/icons.ts`)
- Modify: `i18n/catalogs/{pl,uk,de}.json`: `Condense prompt` (pl `Skondensuj prompt`, uk `Стиснути промпт`, de `Prompt verdichten`), `Prompt` (pl `Prompt`, uk `Промпт`, de `Prompt`), `Condensing…` (pl `Kondensowanie…`, uk `Стискання…`, de `Verdichten…`)
- Modify: `CHANGELOG.md` (Unreleased / Added), `docs/settings.md` (Shortcuts + tray list), `AGENTS.md` (operation.rs bullet: raw values list)
- Test: existing `app/src` tests + `catalogCoverage.test.ts`; add to `shortcuts.test.ts` an assertion that a `Condense prompt` row renders.

- [ ] **Step 1:** Add the shortcuts test assertion; `npm test --prefix app` → FAIL.
- [ ] **Step 2:** Implement the edits above.
- [ ] **Step 3:** `npm test --prefix app`, `npm run check --prefix app`, `python3 scripts/gen-localizations.py check`, `cargo test --workspace --locked` → PASS.
- [ ] **Step 4:** Commit `Show the condense prompt operation in the UI`.
