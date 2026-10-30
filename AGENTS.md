# AGENTS.md

This file provides guidance to AI coding agents (Claude Code, Codex, Cursor, etc.) when working with code in this repository. It is the single source of truth for project instructions; `CLAUDE.md` only points here.

## What this is

LingoPop for Windows: a tray AI translator (translate, fix grammar, rewrite, explain, screen translate, magic) built with Tauri v2 and an OS-agnostic Rust core. It is a sibling of the macOS app `slucheninov/lingopop` and shares its persisted formats. Design spec: [docs/superpowers/specs/2026-10-30-lingopop-app-design.md](docs/superpowers/specs/2026-10-30-lingopop-app-design.md).

## Layout

Each task appends its own modules here in the same commit.

- `Cargo.toml` - workspace (`resolver = "2"`, members `crates/*`, shared package fields, version `0.1.0`)
- `rust-toolchain.toml`, `rustfmt.toml` - toolchain (honoured only by CI locally, no `rustup` on the dev machine) and formatting
- `.gitignore`, `.gitattributes` (`eol=lf`), `LICENSE`, `README.md`, `AGENTS.md`, `CLAUDE.md`
- `scripts/check-core-deps.sh` - guard: `lingopop-core` must not depend on OS or UI crates, nor on a platform TLS stack (`native-tls`, `openssl*`, `aws-lc-*`, `rustls-native-certs`, `rustls-platform-verifier`, `system-configuration*`) and must not have `tokio` as a direct normal dependency (dev-dependency or transitive via reqwest is fine)
- `.github/workflows/ci.yml` - CI (`core` matrix on Windows and macOS, aggregator job `build-and-test`)
- `crates/lingopop-core/Cargo.toml`, `crates/lingopop-core/src/lib.rs` - the OS-agnostic core crate (`VERSION`)
- `crates/lingopop-core/src/operation.rs` - `AIOperation` (frozen raw values, English names)
- `crates/lingopop-core/src/provider.rs` - `ProviderType`, `ProviderField`, static `ProviderCapabilities`, `ProviderConnectionSettings`, `default_providers()`
- `crates/lingopop-core/src/languages.rs` - `LanguageInfo`, the 28-row `ALL` table, `quick_list()`, `info()`, `code()` / `deepl_code()` / `microsoft_code()`, `name_for_detect_code()` (ISO 639-3 detector codes)
- `crates/lingopop-core/src/time.rs` - `Timestamp(f64)`, Apple-reference seconds kept exactly as read (lossless round trip, whole seconds written as JSON integers like Swift, always finite, `-0.0` normalized to `0.0`), plus `Clock`, `SystemClock` and the test-driven `ManualClock`
- `crates/lingopop-core/src/serde_uuid.rs` - `#[serde(with = ...)]` UUID codec (uppercase on write, any case on read)
- `crates/lingopop-core/src/records.rs` - `HistoryEntry`, `UsageRecord` (provider/operation kept as raw strings so unknown values survive a rewrite; typed accessors)
- `crates/lingopop-core/src/envelope.rs` - `MergeableEnvelope<T>` v2 file format (`version`/`items`/`deleted`/`clearedBefore`, legacy bare array and UTF-8 BOM read), `Tombstone`, `Mergeable`, pure `merged` / `pruned` / `mark_deleted` / `mark_cleared` (90-day tombstone retention; uuid case-insensitive, linear time)
- `crates/lingopop-core/src/fsio.rs` - file helpers: `strip_bom` (the crate's single BOM rule), `is_blank`, `read_optional`, `atomic_write` (temp file + `sync_all` + rename, 0600 on Unix when private; std's `rename` replaces an existing file on Windows; transient sharing/permission errors are retried briefly there), `quarantine` (`<name>.bak-<unix seconds>[-n]`; only `NotFound` marks a candidate name free), `LoadOutcome`, `StoreError`
- `crates/lingopop-core/src/paths.rs` - `DataPaths` (file names in the injected data dir: `settings.json`, `history.json`, `usage.json`, `secrets.dat`; the core never computes the dir)
- `crates/lingopop-core/src/store_common.rs` - crate-private `EnvelopeFile<T>`: the one implementation of the store load rule (missing / blank / BOM / legacy / quarantine of corrupt files) and persist rule (prune tombstones, atomic write, then swap memory)
- `crates/lingopop-core/src/history_store.rs` - `HistoryStore` (`history.json`, newest first, cap on `add` / `trim`, `remove` tombstones, `clear` watermark, cap overflow never tombstoned)
- `crates/lingopop-core/src/usage_store.rs` - `UsageStore` (`usage.json`, oldest first, 31-day retention from UTC midnight, `reset` watermark)
- `crates/lingopop-core/src/keyed_map.rs` - `ProviderKeyedMap<V>`: provider-keyed map persisted as a JSON object keyed by raw value; reads the legacy flat array, skips unknown provider keys and undecodable values, errors on a top-level scalar (port of `ProviderKeyedDictionary.swift`)
- `crates/lingopop-core/src/hotkey.rs` - `Hotkey` (W3C `KeyboardEvent.code` key + `Modifier` list: `new` normalizes, `is_valid`, `display_string`, `default_translate` = Ctrl+Alt+T; defaults avoid AltGr letters)
- `crates/lingopop-core/src/prompts.rs` - prompt texts copied verbatim from `PromptBuilder.swift`: `DEFAULT_MAGIC_PROMPT`, `system_prompt` (translate = screenTranslate; magic substitutes every `{{targetLanguage}}`, empty falls back to the default), `user_content` (`<text>` fence with `&` `<` `>` escaped), `sanitize_output` (peels nested fences, then unescapes `&lt;` `&gt;` `&amp;` in that order), `vision_prompt`; line breaks are `\n` escapes, never source newlines
- `crates/lingopop-core/src/settings.rs` - `Settings` (the 12 persisted `SettingsStore.Persisted` fields, per-field lenient decode, `max_input_chars` 0 = no limit), `SettingsError`, `load` (missing/blank = defaults, BOM, unparsable file quarantined, never writes) / `save` (atomic), `sanitize_magic_prompt` (exact Swift rule: ASCII controls dropped except `\n` `\t`, U+007F and non-ASCII kept, 2000 scalars)
- `crates/lingopop-core/src/secrets.rs` - `secrets.dat`: `SecretsPayload` port (flat `{String: String}` plaintext, `apiKey.<raw>` entries plus `providerSettings.v1` JSON string; `decode_payload` / `encode_payload` / `provider_settings_in` -> `ProviderSettingsState` {`Absent`, `Corrupt`, `Loaded`} / `set_provider_settings_in`, unknown entries survive rewrites), the `SecretProtector` trait (DPAPI impl comes in Phase 3; tests use fakes) and `SecretStore` (write = encode, protect, private atomic write, then memory swap; zero-byte file = missing, undecryptable or non-map file quarantined, keys re-entered by the user; `Debug` never prints entries). The mac quarantine-marker / migration machinery is not ported
- `crates/lingopop-core/src/target_language.rs` - `resolve_target_language` (port of `SettingsStore+TargetLanguage.swift`: auto-detect on + known source -> `language_pairs`, else `target_language`; pair values returned as is), `operation_uses_auto_detect` (true only for Translate; Screen Translate detects on the OCR text itself in Phase 4) and `target_language_for`
- `crates/lingopop-core/src/detector.rs` - `detect(text)` with `whatlang` (pure Rust, no model files): restricted to the `languages::ALL` table with a `Detector` allowlist; returns whatlang's best guess mapped through `languages::name_for_detect_code` (not gated on `is_reliable()`, matching the mac `NLLanguageRecognizer.dominantLanguage`); `None` for empty / whitespace / digits / punctuation only or a language outside the table
- `crates/lingopop-core/src/action_row.rs` - `ActionRowMode` (`Full` / `Mixed` / `Compact`, `is_icon_only`, `preferred`, `candidates`), `LONG_TEXT_CHARACTER_THRESHOLD` (120) and `row_width` (port of `ActionRowMetrics`; the text measurer is injected, widths rounded up like Swift `ceil`)
- `crates/lingopop-core/src/errors.rs` - `TranslationError` (`user_summary` as a `UserMessage` = English catalog key + args, `user_detail` as a list of such lines joined by `english_lines`, the HTTP URL is redacted inside `user_detail`; every key exists in the mac catalog except `VERBATIM_KEY` pass-through lines, checked by a local-only test; `Display` = summary + detail), `UserMessage::english`, `FallbackInfo`, `redacted_url` (credentials, query, fragment stripped; `<redacted-url>` when unparsable) and `sanitized_http_body_detail` (response bodies are never echoed); hand-written `Debug` prints an HTTP body as its character count only; no error text ever contains an API key
- `crates/lingopop-core/src/http/mod.rs` - the async HTTP seam (port of `ProviderHTTP.swift`): `Transport` trait (`async_trait`; transport failures are `TranslationError::Network`), `HttpMethod`, `HttpRequest` (`get`, case-insensitive `header`, `json_body`; hand-written `Debug` redacts `x-api-key` / `authorization` / `x-goog-api-key` / `ocp-apim-subscription-key`, prints the URL without its query and the body as a length only), `HttpResponse`, `perform` (2xx = body, anything else `TranslationError::Http` with a redacted URL, no retries), `json_post` (Content-Type first, compact UTF-8 JSON), `decode` (its `Decoding` text is only `Unexpected response shape` / `Invalid JSON` plus serde's line and column, never serde's message, which quotes response values), `mime_type` (JPEG by `FF D8 FF`, else PNG). `tokio` is a dev-dependency only
- `crates/lingopop-core/src/http/reqwest_transport.rs` - `ReqwestTransport` (`new`, `without_system_proxy` for loopback tests so `HTTP_PROXY` cannot interfere), `HttpTimeouts` (default connect 30 s / read 30 s / total 60 s), `redirect_allowed`: reqwest 0.12.28 with rustls + webpki roots + ring only (no platform TLS; `system-proxy` only on Windows, see `Cargo.toml`), User-Agent `LingoPop/<VERSION>` (a request header overrides it), redirects followed only to the same origin as the original request (scheme, host case-insensitive, effective port) and cut at 10 previous URLs, header errors name only the header (never the value), `Network` text is the reqwest message without URL plus its source chain. reqwest `>=0.13` is ignored in `.github/dependabot.yml` (needs the platform verifier and aws-lc-rs)
- `crates/lingopop-core/tests/http_transport.rs` - 12 loopback tests (`wiremock` on 127.0.0.1, random ports)
- `crates/lingopop-core/src/http/endpoint.rs` - `endpoint(base_url, default_base_url, path, query)`: exact `ProviderHTTP.endpoint` semantics (trim, empty = default, HTTPS only, scheme checked before host, userinfo / query / fragment of the base dropped, trailing slashes of the base path stripped, form-encoded query pairs)
- `crates/lingopop-core/src/http/fake.rs` - `FakeTransport` (always compiled, reused by Phases 3-4): closure handler, `json(status, body)`, `by_host(routes)`, `requests()` in order
- `crates/lingopop-core/src/providers/mod.rs` - the provider contract: `ProviderConfig` (hand-written `Debug` prints `api_key: "<redacted>"`), `TokenUsage`, `ProviderResult` (`text`, `usage`, `truncated`), `AIProvider` (`async_trait`; `execute` and `execute_vision` take the `Transport` explicitly; the default `execute_vision` is `OperationNotSupported{provider, ScreenTranslate}`; capabilities stay in `ProviderType::capabilities()`, not in the trait), `ProviderRegistry` (`standard`, `empty`, `insert`, `get` -> `ProviderNotFound`) and the crate-private helpers `finish_text` (sanitize or trim, empty = `EmptyResponse`), `require_api_key`, `encode_image` (sniffed MIME + standard padded base64)
- `crates/lingopop-core/src/providers/claude.rs` - `ClaudeProvider` (`POST {base}/v1/messages`, headers `x-api-key` + `anthropic-version: 2023-06-01`, `max_tokens` 4096; text ops send the `system` prompt and a `<text>` fenced user block and sanitize the reply; vision sends the image block first then the raw vision prompt, no `system`, checks the API key first and only trims; reply blocks concatenated without separator, `stop_reason == "max_tokens"` = truncated). Fixtures: `crates/lingopop-core/tests/fixtures/providers/claude/`
- `crates/lingopop-core/src/providers/openai.rs` - `OpenAiCompatible { provider_type }` with `openai()` and `grok()` constructors (Grok is the same wire format at `https://api.x.ai`; the default base comes from `ProviderType::capabilities()`). `POST {base}/v1/chat/completions` (no `/v1` dedup), headers `Content-Type` + `Authorization: Bearer <key>`, body only `model` + `messages` (no `max_tokens`/`temperature`/`stream`). Text ops send a `system` message and a `<text>` fenced `user` string and sanitize the reply; vision sends one `user` message with the `image_url` data-URI part first then the raw vision prompt, checks the API key first and only trims. Only `choices[0].message.content` is read: no choice or `null` content = `EmptyResponse`, array content = `Decoding`; `finish_reason == "length"` = truncated; `prompt_tokens`/`completion_tokens` = usage. Registered for both `OpenAI` and `Grok` in `ProviderRegistry::standard()`. Fixtures: `crates/lingopop-core/tests/fixtures/providers/{openai,grok}/`
- `crates/lingopop-core/src/providers/gemini.rs` - `GeminiProvider` (`POST {base}/v1beta/models/<model>:generateContent`, no query string, no streaming; headers `Content-Type` + `x-goog-api-key`; the model name is validated by the crate-private `is_valid_model` (`[A-Za-z0-9][A-Za-z0-9._-]*`, hand-written, a trailing newline is rejected) before the URL is built, for text and vision, else `InvalidConfiguration("Gemini model name is empty or contains unsupported characters")`; vision checks the API key first. Body is only `systemInstruction` (text ops) + `contents` (no `generationConfig`, no `safetySettings`); vision sends the `inlineData` part first, then the raw vision prompt, no `systemInstruction`, and only trims the reply. `candidates` is required (a blocked prompt without it is `Decoding`); every candidate's text parts are concatenated without separator, a candidate without content or an empty list = `EmptyResponse`; first candidate `finishReason == "MAX_TOKENS"` = truncated; `promptTokenCount` is required in `usageMetadata`, `candidatesTokenCount` defaults to 0, `thoughtsTokenCount` is ignored). Registered in `ProviderRegistry::standard()`. Fixtures: `crates/lingopop-core/tests/fixtures/providers/gemini/`
- `crates/lingopop-core/src/providers/google_free.rs` - `GoogleFreeProvider` (unofficial `GET {base}/translate_a/single?client=gtx&sl=auto&tl=<code>&dt=t&q=<text>` in this query order, only header `User-Agent: Mozilla/5.0`, no key, no body; `tl` = `languages::code(target)`; the query is form-encoded by `endpoint`, so `+` `&` `%` in the text survive: a deliberate fix, Swift sends `+` raw and Google reads it as a space). Ignores the operation (Screen Translate = Translate on the OCR text; the runner rejects unsupported operations). Reply parsed with `serde_json::Value`: not JSON = `Decoding("Invalid JSON")`, top level or its first element not an array = `Decoding("Unexpected response shape")`, the first-string element of every array segment is concatenated, then trimmed, and only then checked for emptiness (`EmptyResponse`); `usage` None, `truncated` false. An HTTP error carries the URL without its query, so the user text never appears in an error. Fixture: `crates/lingopop-core/tests/fixtures/providers/google_free/`
- `crates/lingopop-core/src/providers/google_cloud.rs` - `GoogleCloudProvider` (Cloud Translation v2: `POST {base}/language/translate/v2`, no `?key=`; headers `Content-Type`, `X-goog-api-key`, and `Referer` only when `config.referer` is set and non-empty; body `{q, target, model, format: "text"}` with `model` omitted when the config model is empty; the API key is not checked up front, like Swift). Reply `data.translations[0].translatedText` (required; `data` or `translations` missing, e.g. a 200 error object, = `Decoding`); no first translation or an empty string = `EmptyResponse`, checked before the trim (so `"  "` returns `""`); `usage` None, `truncated` false. A test pins that an HTTP error's `Display` / `Debug` never contain the key. Both providers are registered in `ProviderRegistry::standard()`. Fixture: `crates/lingopop-core/tests/fixtures/providers/google_cloud/`
- `SECURITY.md`, `PRIVACY.md`, `CONTRIBUTING.md`, `CHANGELOG.md` - repository meta files (adapted from the mac repo for Windows: `%APPDATA%\LingoPop\`, DPAPI, no sync)
- `docs/api-keys.md` - per-provider API key guide; the headings `Claude`, `OpenAI`, `Gemini`, `Grok`, `Google Free`, `Google Translate`, `DeepL`, `Microsoft Translator` are link anchors (`ProviderType.keyGuideURL` in the app, Phase 4) and must keep their names; `docs/icon.png` is a copy of the mac repo icon
- `.github/CODEOWNERS`, `.github/pull_request_template.md`, `.github/ISSUE_TEMPLATE/`, `.github/dependabot.yml` - repo templates and Dependabot (`cargo` and `github-actions`; the `npm` entry for `/app` is added in Phase 4 together with `app/package.json`)
- `.coderabbit.yaml` - CodeRabbit review config (Rust and frontend path instructions, shellcheck / actionlint / gitleaks / markdownlint / yamllint)
- `scripts/mac-fixtures/run.sh`, `scripts/mac-fixtures/main.swift` - compiles the real macOS model files with `swiftc` and generates / checks the compatibility fixtures (macOS only)
- `crates/lingopop-core/tests/fixtures/mac/` - generated, committed fixtures and goldens (`history.json`, `history.legacy.json`, `usage.json`, `secrets.payload.json`, `languages.golden.json`, `prompts.golden.json`, `SOURCE.txt` with the mac commit); never hand-edited
- `crates/lingopop-core/tests/mac_fixtures.rs` - checks the Rust types against those fixtures (history, usage, languages, prompts, secrets payload; plus the ignored writer for the Swift reverse check)

## Build / test

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
bash scripts/check-core-deps.sh
```

Windows is verified only by CI; the dev machine is a Mac.

### Mac fixtures

The fixtures under `crates/lingopop-core/tests/fixtures/mac/` come from the real macOS models, not from Rust. macOS with the Swift toolchain only; the mac repo is read from `$LINGOPOP_MAC_REPO` (default `~/Works/slava/lingo-pop`) and never modified.

```bash
./scripts/mac-fixtures/run.sh generate    # regenerate fixtures + SOURCE.txt (only when the mac models change)
cargo test -p lingopop-core --test mac_fixtures
# manual gate before changing the history / usage / secrets formats (Rust-written files must decode in Swift):
cargo test -p lingopop-core --test mac_fixtures -- --ignored write_rust_outputs_for_swift_check
./scripts/mac-fixtures/run.sh check target/mac-roundtrip
```

`check <dir>` decodes `history.json`, `usage.json` and `secrets.payload.json` with the Swift models, skips absent files and exits 1 on a decode failure or when all three are absent. The roundtrip directory defaults to `target/mac-roundtrip` (override with `LINGOPOP_ROUNDTRIP_DIR`).

## Rules

- The core stays OS-agnostic: no `windows`, `winapi`, `tauri*`, `objc*`, `cocoa`, `core-foundation*` dependencies, and `#![forbid(unsafe_code)]`. `scripts/check-core-deps.sh` enforces it in CI.
- Persisted raw values and the date/UUID encoding are frozen and must never be renamed or changed:
  - `ProviderType`: `"Claude"`, `"OpenAI"`, `"Gemini"`, `"Grok"`, `"Google (Free)"`, `"Google Translate"`, `"DeepL"`, `"Microsoft Translator"`.
  - `AIOperation`: `"translate"`, `"grammarFix"`, `"rewrite"`, `"explain"`, `"screenTranslate"`, `"magic"`.
  - Dates are JSON numbers: `f64` seconds since 2001-01-01T00:00:00Z (add 978307200 for Unix epoch).
  - UUIDs are written uppercase and read case-insensitively.
  - `history.json` / `usage.json` use the envelope v2 `{ "version": 2, "items": [...], "deleted": [{"id","at"}], "clearedBefore": <date> }`.
  - UI and error text use `display_name()`, never the raw value.
- Prompt texts are copied verbatim from the macOS `Core/PromptBuilder.swift` (token `{{targetLanguage}}`) and are never localized.
- No external network in tests; loopback mock servers in `tests/` are allowed. Live tests are `#[ignore]` and never run in CI.
- No secrets or API keys in logs, fixtures or issues.

## Mac compatibility

Compatibility fixtures are generated by `scripts/mac-fixtures/run.sh` and are never hand-edited.

## Branching

Until Phase 1 Task 12 is done, commits go directly to `main`. Afterwards `main` is PR-only (branch ruleset) with the required checks `build-and-test` and `CodeRabbit`, one approving review and resolved review threads; direct pushes are rejected.
