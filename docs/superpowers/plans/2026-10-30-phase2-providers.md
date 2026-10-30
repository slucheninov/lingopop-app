# Phase 2: HTTP client, providers and operation runner — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add to `crates/lingopop-core` an async HTTP transport seam (with a reqwest implementation and a fake), the eight providers with the same wire behaviour as the macOS app, and the operation runner with a single fallback; plus live smoke tests that the owner runs on a Mac.

**Architecture:** Providers never touch the network directly: they build an `HttpRequest`, hand it to an injected `&dyn Transport`, and parse the bytes. `ReqwestTransport` (rustls + webpki roots + ring, no platform verifier) is the production transport; `FakeTransport` records requests and returns canned responses for every provider and runner test. Capabilities stay single-sourced in `ProviderType::capabilities()`; `OperationRunner` validates, executes and falls back exactly like `Core/OperationRunner.swift`. Core stays OS-agnostic and has no `tokio` dependency of its own.

**Tech Stack:** Rust stable (edition 2021), `reqwest` 0.12 (no default features), `async-trait`, `base64`, existing `serde` / `serde_json` / `url` / `thiserror`; dev: `tokio`, `wiremock`, `tempfile`.

**Spec:** `docs/superpowers/specs/2026-10-30-lingopop-app-design.md` (Russian): section 5 "Providers" and "Operations и fallback", section 9a (provider tests against mocks incl. 429/5xx fallback; live smoke tests `#[ignore]`, run by the owner on a Mac, never in CI). macOS reference app (read-only): `/Users/lucheninov/Works/slava/lingo-pop` at v0.4.0, commit `049acf3` ("the mac repo"; paths like `Providers/ClaudeProvider.swift` are relative to it).

## Roadmap

- **Phase 1 (done):** repo bootstrap + CI + `lingopop-core` foundations.
- **Phase 2 (this plan):** HTTP client + 8 providers + operation runner with fallback (`Core/OperationRunner.swift`) + live smoke tests (`#[ignore]`).
- **Phase 3:** `crates/lingopop-platform` traits, `fake`, `windows` impls, `macos` dev impl, Windows smoke tests.
- **Phase 4:** Tauri app (orchestration, popup, settings, history, tray, overlay), localization, usage aggregation. The app calls `OperationRunner::run` / `run_vision` and persists `TranslationResult::history_entry` / `usage_record`.
- **Phase 5:** NSIS installer, `release.yml`, publish-release script, `RELEASING.md`, update checker, docs polish.

Phase 2 runs on branch `phase2-providers` (created from `main` at `3e6acb4`). Every task commits on it; Task 9 opens ONE pull request to `main`. `main` is PR-only (ruleset: `build-and-test` + CodeRabbit required, 1 approval); the Phase 1 plan asked this first Phase 2 PR to say so in its description.

## Global Constraints

- `lingopop-core` is OS-agnostic with `#![forbid(unsafe_code)]`; the guard `scripts/check-core-deps.sh` runs in CI on windows and macos.
- `tokio` is never a direct normal dependency of `lingopop-core` (it is only a dev-dependency; reqwest pulls it transitively).
- TLS is rustls with webpki roots and the ring provider only. Forbidden anywhere in the normal tree: `native-tls`, `openssl`, `openssl-sys`, `aws-lc-rs`, `aws-lc-sys`, `rustls-native-certs`, `rustls-platform-verifier`, `core-foundation*`, `system-configuration*`.
- No external network in tests; loopback mock servers (`wiremock` on 127.0.0.1) in `tests/` are allowed. Live tests are `#[ignore]` and never run in CI.
- An API key never appears in `Debug` output, error text, logs or fixtures (fixtures use fake keys such as `sk-ant-FIXTURE`, `AIza-FIXTURE`).
- Persisted raw values are frozen (`ProviderType`, `AIOperation`); user-visible text uses `display_name()`, never the raw value.
- Prompt texts come only from `prompts.rs` (`system_prompt`, `user_content`, `sanitize_output`, `vision_prompt`); providers never re-type them.
- One pull request from `phase2-providers`; `Cargo.lock` is committed with every dependency change; CI uses `--locked`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` must pass on windows and macos.
- Each task appends its modules to the Layout section of `AGENTS.md` in the same commit. Conventional commits, no `Co-Authored-By` / "Generated with" footers.
- `cargo test` takes one positional filter: every step uses the whole crate, `--test <name>` (whole binary) or ONE filter; never several.
- Task gate (run at the end of every task, all four must pass): `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`.

## Review Focus

Five failure modes the spec implies but does not spell out, most likely first. Each has a pinning test in the owning task (marked `(RF n)`).

1. **API keys leaking** via `Debug` of `HttpRequest` / `ProviderConfig` / `ConfigSnapshot`, via reqwest error text (which can embed the URL) or via an invalid-header error. Tests: Task 1 `http_request_debug_redacts_secret_headers`, Task 2 `network_error_message_never_contains_request_url`, `invalid_header_value_error_does_not_leak_it`, Task 3 `provider_config_debug_redacts_api_key`, Task 8 `config_snapshot_debug_redacts_api_keys`.
2. **Non-ASCII and special text** (Cyrillic, emoji, `&<>`, `+`, `%`, newlines) through the Google Free query and the JSON bodies. Tests: Task 6 `google_free_encodes_special_characters_in_query`, Task 3 `claude_user_text_with_markup_is_escaped_in_body`.
3. **Custom base URLs** (LiteLLM proxy with a path, trailing slash, explicit port, surrounding whitespace). Tests: Task 1 `endpoint_*`, plus one `<provider>_custom_base_with_path_trailing_slash_and_port` per provider.
4. **Fallback misconfiguration** (fallback equal to primary, without a key, not supporting the operation, also failing). Tests: Task 8.
5. **HTTP 200 with an unexpected body** (error object, Gemini blocked prompt without candidates, OpenAI null content / empty choices) mapping to `Decoding` versus `EmptyResponse` exactly like Swift. Tests: Tasks 3-7 `*_error_object_body_is_decoding`, `gemini_blocked_prompt_without_candidates_is_decoding`, `openai_null_content_is_empty_response`, `openai_empty_choices_is_empty_response`.

## Dependency decisions (verified while planning)

- **reqwest 0.12.28, not 0.13.** 0.13's TLS features (`rustls`, `rustls-no-provider`) both enable `rustls-platform-verifier` (pulls `core-foundation` on macOS) and the default provider is aws-lc-rs, which Global Constraints forbid. 0.12's `rustls-tls-webpki-roots` = webpki roots + ring. `.github/dependabot.yml` ignores `reqwest >= 0.13.0` (Task 2).
- **System proxy.** reqwest's `system-proxy` feature pulls `system-configuration` -> `core-foundation` on macOS. It is therefore enabled only for Windows through a `[target.'cfg(windows)'.dependencies]` entry (Windows registry proxy settings, parity with URLSession on the shipped platform). On macOS (dev only) the `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` environment variables still work. Verified with a scratch crate: macOS tree has none of the forbidden crates, the Windows tree adds only `windows-registry` and friends (the guard only forbids the name `windows`).

## File Structure

```
crates/lingopop-core/
  Cargo.toml                               (Tasks 1, 2, 3)
  src/lib.rs                               module decls + re-exports (Tasks 1, 3, 8)
  src/http/mod.rs                          HttpMethod, HttpRequest, HttpResponse, Transport, perform, json_post, decode, mime_type   (Task 1)
  src/http/endpoint.rs                     endpoint()                                                                          (Task 1)
  src/http/fake.rs                         FakeTransport                                                                       (Task 1)
  src/http/reqwest_transport.rs            ReqwestTransport, HttpTimeouts, redirect_allowed                                    (Task 2)
  src/providers/mod.rs                     ProviderConfig, TokenUsage, ProviderResult, AIProvider, ProviderRegistry, shared helpers  (Task 3)
  src/providers/{claude,openai,gemini,google_free,google_cloud,deepl,microsoft}.rs                                              (Tasks 3-7)
  src/runner.rs                            ConfigSnapshot, RunRequest, TranslationResult, OperationRunner, should_retry_with_fallback  (Task 8)
  tests/http_transport.rs                  wiremock loopback tests                                                             (Task 2)
  tests/live.rs                            #[ignore] live smoke tests                                                          (Task 9)
  tests/fixtures/providers/<provider>/*.json   hand-written realistic responses                                                (Tasks 3-7)
  tests/fixtures/vision/hello.png          generated once on a Mac                                                             (Task 9)
scripts/check-core-deps.sh                 extended forbidden list (Task 1)
scripts/make-vision-fixture.swift, scripts/live-tests.sh   (Task 9)
```

---

### Task 1: `http` core, `FakeTransport` and the dependency guard

**Files:**
- Create: `crates/lingopop-core/src/http/{mod.rs, endpoint.rs, fake.rs}`
- Modify: `crates/lingopop-core/src/lib.rs` (`pub mod http;`), `crates/lingopop-core/Cargo.toml` (`async-trait`; dev: `tokio` features `macros, rt, rt-multi-thread, time`), `Cargo.lock`, `scripts/check-core-deps.sh`, `AGENTS.md` (Layout; Rules: replace "No network in unit tests." with "No external network in tests; loopback mock servers in `tests/` are allowed. Live tests are `#[ignore]` and never run in CI.")

**Interfaces:**
- Consumes: `errors::{TranslationError, redacted_url}`.
- Produces (all `pub` in `lingopop_core::http`):
  - `enum HttpMethod { Get, Post }` (`Debug, Clone, Copy, PartialEq, Eq`).
  - `struct HttpRequest { pub method: HttpMethod, pub url: url::Url, pub headers: Vec<(String, String)>, pub body: Option<Vec<u8>> }` (`Clone`; hand-written `Debug`); `HttpRequest::get(url: Url, headers: &[(&str, &str)]) -> HttpRequest`, `header(&self, name: &str) -> Option<&str>` (case-insensitive), `json_body(&self) -> serde_json::Value` (`Null` when absent or not JSON).
  - `struct HttpResponse { pub status: u16, pub body: Vec<u8> }` (`Debug, Clone, PartialEq`); `HttpResponse::new(status: u16, body: impl Into<Vec<u8>>) -> Self`.
  - `#[async_trait] trait Transport: Send + Sync { async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TranslationError>; }` (transport failures are `TranslationError::Network(String)`).
  - `async fn perform(transport: &dyn Transport, request: HttpRequest) -> Result<Vec<u8>, TranslationError>`.
  - `fn endpoint(base_url: &str, default_base_url: &str, path: &str, query: &[(&str, &str)]) -> Result<Url, TranslationError>`.
  - `fn json_post(url: Url, headers: &[(&str, &str)], body: &impl Serialize) -> Result<HttpRequest, TranslationError>`; `fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, TranslationError>`; `fn mime_type(image: &[u8]) -> &'static str`.
  - `FakeTransport` (always compiled; Phases 3-4 reuse it): `new(handler: impl Fn(&HttpRequest) -> Result<HttpResponse, TranslationError> + Send + Sync + 'static)`, `json(status: u16, body: &str)` (fixed response), `by_host(routes: Vec<(&str, HttpResponse)>)` (match on `url.host_str()`; unknown host -> `Network("no route")`), `requests(&self) -> Vec<HttpRequest>` (every request, in order). Implements `Transport`.

- [ ] **Step 1: Write the failing tests**

`http/endpoint.rs` (`#[cfg(test)]`, exact Swift semantics from `ProviderHTTP.endpoint`; default base `https://api.example.com`, path `/v1/x` unless stated):
```rust
// endpoint_uses_default_base_when_empty            ("" -> "https://api.example.com/v1/x")
// endpoint_whitespace_only_base_uses_default       ("  \n " -> default)
// endpoint_trims_whitespace_around_base            ("  https://llm.example.com  " -> "https://llm.example.com/v1/x")
// endpoint_rejects_http_scheme                     ("http://llm.example.com" == Err(InvalidConfiguration("baseURL must use HTTPS scheme".into())))
// endpoint_scheme_check_is_case_insensitive        ("HTTPS://Example.COM" -> "https://example.com/v1/x")
// endpoint_unparseable_base_is_invalid_configuration ("not a url", "//h" -> same InvalidConfiguration as above)
// endpoint_empty_host_is_invalid_url               ("https://" and "https://:443" -> Err(InvalidUrl))
// endpoint_http_with_empty_host_is_invalid_configuration ("http://" -> InvalidConfiguration: Swift checks the scheme first)
// endpoint_drops_userinfo_query_and_fragment       ("https://u:pw@h.example.com/p/?a=1#f" -> "https://h.example.com/p/v1/x")
// endpoint_keeps_explicit_port                     ("https://h.example.com:8443" -> "https://h.example.com:8443/v1/x"; "https://[::1]:8443" keeps brackets and port)
// endpoint_normalizes_default_port                 ("https://h.example.com:443" -> "https://h.example.com/v1/x": the url crate drops :443; Swift would keep it, equivalent in practice)
// endpoint_strips_all_trailing_slashes_of_base_path ("https://p.example.com/litellm///" -> "https://p.example.com/litellm/v1/x")
// endpoint_keeps_base_path                         ("https://p.example.com/a/b" -> "https://p.example.com/a/b/v1/x")
// endpoint_prepends_slash_to_path                  (path "v1/x" == path "/v1/x")
// endpoint_keeps_colon_in_path                     (path "/v1beta/models/gemini-3.8-flash:generateContent" stays unescaped)
// endpoint_without_query_has_no_question_mark      (url.query() == None)
// endpoint_query_pairs_are_form_encoded            ([("q","a b&c+d")] -> query "q=a+b%26c%2Bd"; order of pairs preserved)
```
`http/mod.rs` tests (`#[tokio::test]` for async):
```rust
// perform_returns_body_for_200_204_and_299
// perform_maps_non_2xx_to_http_error               (300, 302, 404, 429, 500: Err(Http{status, body: "boom", url}))
// perform_non_utf8_body_becomes_empty_string       (404 + [0xFF,0xFE] -> body "")
// perform_redacts_url_in_http_error                (request https://api.x.com/v1?q=SECRET -> url "https://api.x.com/v1")
// perform_propagates_transport_error               (Network("down") unchanged)
// decode_ok_and_decode_failure_is_decoding_error   (valid -> Ok; "not json", missing key, wrong type -> Err(Decoding(_)) with non-empty text)
// json_post_sets_content_type_first_then_headers   (headers == [("Content-Type","application/json"), ("x-api-key","k")]; method Post)
// json_post_body_is_compact_utf8_json              (json!({"a":"Привет"}) -> bytes of {"a":"Привет"})
// mime_type_follows_swift_rule                     ([0x89,0x50,0x4E,0x47] -> png; [0xFF,0xD8,0xFF,0xE0] -> jpeg; [0xFF,0xD8,0xFF] (3 bytes) -> png; b"GIF89a" -> png; [] -> png)
// http_request_debug_redacts_secret_headers        (RF 1: x-api-key, Authorization, X-Goog-Api-Key, ocp-apim-subscription-key in any case -> "<redacted>"; Content-Type and Referer shown; body printed as its length only; the URL query is not printed; "sk-ant-FIXTURE" appears nowhere)
// http_request_header_lookup_is_case_insensitive
```
`http/fake.rs` tests: `fake_records_requests_in_order`, `fake_json_returns_fixed_response`, `fake_by_host_routes_and_unknown_host_is_network_error`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo add -p lingopop-core async-trait && cargo add -p lingopop-core --dev tokio --features macros,rt,rt-multi-thread,time`, then `cargo test -p lingopop-core http::`
Expected: FAIL to compile (module `http` missing). The first command updates `Cargo.lock` (run without `--locked`).

- [ ] **Step 3: Implement the module**

- `perform`: 200..=299 returns the body; otherwise `Http { status, body: String::from_utf8(body).unwrap_or_default(), url: redacted_url(request.url.as_str()) }`; no per-status branching, no retries.
- `HttpRequest` `Debug`: `HttpRequest { method, url: <redacted_url>, headers: [(name, value or "<redacted>")], body_len: Option<usize> }`; the secret names (case-insensitive) are `x-api-key`, `authorization`, `x-goog-api-key`, `ocp-apim-subscription-key`.
- `json_post`: `serde_json::to_vec`; a serialization failure maps to `Decoding(err.to_string())` (cannot happen for our structs). `decode`: any `serde_json` error -> `Decoding(err.to_string())`.
- `mime_type`: `len < 4` -> png; starts with `89 50 4E 47` -> png; starts with `FF D8 FF` -> jpeg; anything else png.
- `endpoint` (the algorithm the tests above do not determine):
```rust
let trimmed = base_url.trim();
let base = if trimmed.is_empty() { default_base_url } else { trimmed };
let mut url = match Url::parse(base) {
    Ok(u) => u,
    // Swift checks the scheme before the host: "http://" is a configuration error, "https://" an invalid URL.
    Err(url::ParseError::EmptyHost) if scheme_of(base).eq_ignore_ascii_case("https") => return Err(TranslationError::InvalidUrl),
    Err(_) => return Err(https_only()),
};
if url.scheme() != "https" { return Err(https_only()); }   // https_only() = InvalidConfiguration("baseURL must use HTTPS scheme")
if url.host_str().map_or(true, str::is_empty) { return Err(TranslationError::InvalidUrl); }
let base_path = url.path().trim_end_matches('/').to_owned();
let extra = if path.starts_with('/') { path.to_owned() } else { format!("/{path}") };
let _ = url.set_username(""); let _ = url.set_password(None);
url.set_query(None); url.set_fragment(None);
url.set_path(&format!("{base_path}{extra}"));
if !query.is_empty() { url.query_pairs_mut().extend_pairs(query); }
```
  (`scheme_of` = the text before the first `:`). Pinned by scratch run: `Url::parse("https://")` and `"https://:443"` fail with `EmptyHost`, `"not a url"` with `RelativeUrlWithoutBase`, `"https:///path"` parses with host `path` (not tested, WHATWG behaviour).
- `FakeTransport`: handler in a `Box<dyn Fn ...>`, requests in a `Mutex<Vec<HttpRequest>>`.
- Extend `scripts/check-core-deps.sh` part (b): also fail when the crate name equals `native-tls`, `openssl`, `openssl-sys`, `aws-lc-rs`, `aws-lc-sys`, `rustls-native-certs`, `rustls-platform-verifier` or starts with `system-configuration` (`core-foundation` is already covered).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core http::`
Expected: PASS (about 31 tests).

- [ ] **Step 5: Prove the guard rejects the new names**

Run: `cargo add -p lingopop-core native-tls && bash scripts/check-core-deps.sh; echo "exit=$?"`
Expected: error naming `native-tls`, `exit=1`. Undo: `cargo remove -p lingopop-core native-tls && cargo build -p lingopop-core` (drops the entries from `Cargo.lock`), then `bash scripts/check-core-deps.sh; echo "exit=$?"` expected `exit=0`.

- [ ] **Step 6: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add http transport seam, endpoint builder and FakeTransport"
```

---

### Task 2: `ReqwestTransport` and loopback transport tests

**Files:**
- Create: `crates/lingopop-core/src/http/reqwest_transport.rs`, `crates/lingopop-core/tests/http_transport.rs`
- Modify: `crates/lingopop-core/src/http/mod.rs` (`mod reqwest_transport; pub use reqwest_transport::{ReqwestTransport, HttpTimeouts, redirect_allowed};`), `crates/lingopop-core/Cargo.toml`, `Cargo.lock`, `.github/dependabot.yml`, `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 1 `Transport`, `HttpRequest`, `HttpResponse`, `HttpMethod`.
- Produces: `struct HttpTimeouts { pub connect: Duration, pub read: Duration, pub total: Duration }` (`Debug, Clone, Copy`; `Default` = 30 s / 30 s / 60 s, parity with URLSession 30 s request / 60 s resource); `struct ReqwestTransport` with `new(timeouts: HttpTimeouts) -> Result<Self, TranslationError>` and `without_system_proxy(timeouts: HttpTimeouts) -> Result<Self, TranslationError>` (same client with `no_proxy()`; used by the loopback tests so a developer's `HTTP_PROXY` cannot swallow 127.0.0.1 requests), implementing `Transport`; `pub fn redirect_allowed(original: &Url, target: &Url) -> bool`.

- [ ] **Step 1: Add dependencies**

Run: `cargo add -p lingopop-core reqwest@0.12.28 --no-default-features --features rustls-tls-webpki-roots,http2 && cargo add -p lingopop-core --dev wiremock`, then edit `crates/lingopop-core/Cargo.toml` and add
```toml
[target.'cfg(windows)'.dependencies]
reqwest = { version = "0.12.28", default-features = false, features = ["system-proxy"] }
```
Add to `.github/dependabot.yml` under the `cargo` entry `ignore: [{ dependency-name: "reqwest", versions: [">=0.13.0"] }]` (0.13 needs the platform verifier and aws-lc-rs, see Dependency decisions). Run `cargo build -p lingopop-core` (no `--locked`) so `Cargo.lock` updates.

- [ ] **Step 2: Verify the dependency tree**

Run:
```bash
for c in native-tls openssl openssl-sys aws-lc-rs aws-lc-sys rustls-native-certs rustls-platform-verifier core-foundation system-configuration; do cargo tree -p lingopop-core -e normal -i "$c" 2>&1 | head -1; done
cargo tree -p lingopop-core -e normal -i ring | head -3
cargo tree -p lingopop-core -e normal -i webpki-roots | head -3
cargo tree -p lingopop-core -e normal --target x86_64-pc-windows-msvc -i windows-registry | head -3
bash scripts/check-core-deps.sh
```
Expected: each forbidden crate prints `error: package ID specification ... did not match any packages` (absence is the pass condition); `ring` and `webpki-roots` resolve through `reqwest`; `windows-registry` resolves only for the Windows target; the guard exits 0. If any forbidden crate shows up, stop and fix the features before writing code.

- [ ] **Step 3: Write the failing tests**

Unit tests in `reqwest_transport.rs`: `redirect_allowed_same_origin`, `redirect_allowed_ignores_host_case`, `redirect_allowed_treats_explicit_default_port_as_equal` (ruling: `https://h:443/a` vs `https://h/b` is true, unlike Swift's raw optional compare), `redirect_refused_for_other_host`, `redirect_refused_for_other_port`, `redirect_refused_for_scheme_change` (https -> http), `http_timeouts_default_is_30_30_60`.

`tests/http_transport.rs` (`#[tokio::test]`, `wiremock::MockServer` on 127.0.0.1 over http, `ReqwestTransport::without_system_proxy`; ReqwestTransport does not enforce https, `endpoint` does):
```rust
// success_200_passes_status_body_and_request_details   (server sees method POST, path, header x-api-key, JSON body; response body bytes returned; perform() gives Ok(body))
// default_user_agent_is_lingopop_with_crate_version    (server sees "LingoPop/<lingopop_core::VERSION>")
// request_user_agent_overrides_default                 (GET with User-Agent "Mozilla/5.0" -> server sees exactly that)
// server_error_500_becomes_http_error_with_body        (perform() -> Http{500, body "boom", url without query})
// not_found_with_non_utf8_body_gives_empty_body        (404 + [0xFF,0xFE] -> Http{404, body ""})
// same_origin_redirect_is_followed_and_keeps_headers   (302 /a -> /b on the same server; final 200; /b saw x-api-key)
// cross_origin_redirect_is_refused_and_not_contacted   (server A 302 -> server B (other port); result Http{302, ..}; B received zero requests)
// redirect_loop_is_cut_after_ten_hops                  (/loop 302 -> /loop; result Http{302}; server received at most 11 requests)
// slow_response_times_out_as_network_error             (HttpTimeouts{connect 1s, read 200ms, total 500ms}; delay 2s -> Network(_))
// connection_refused_is_network_error                  (bind 127.0.0.1:0, read the port, drop the listener)
// network_error_message_never_contains_request_url     (RF 1: refused request to /secret-path?q=SECRETTEXT -> message contains neither "secret-path" nor "SECRETTEXT"; non-empty)
// invalid_header_value_error_does_not_leak_it          (RF 1: header "x-api-key" = "sk-secret\nx" -> Err(InvalidConfiguration(_)) whose Display/Debug lack "sk-secret")
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p lingopop-core --test http_transport`
Expected: FAIL to compile (`ReqwestTransport` missing).

- [ ] **Step 5: Implement `reqwest_transport.rs`**

- Client: `reqwest::Client::builder().use_rustls_tls()` is not needed (single TLS backend); set `connect_timeout`, `read_timeout`, `timeout` from `HttpTimeouts`, `user_agent(format!("LingoPop/{}", crate::VERSION))`, `redirect(Policy::custom(..))`; `without_system_proxy` adds `.no_proxy()`. A builder failure maps to `Network(err.without_url().to_string())`.
- `redirect_allowed(original, target)`: scheme and host compared case-insensitively, `port_or_known_default()` compared for equality:
```rust
original.scheme().eq_ignore_ascii_case(target.scheme())
    && original.host_str().map(str::to_ascii_lowercase) == target.host_str().map(str::to_ascii_lowercase)
    && original.port_or_known_default() == target.port_or_known_default()
```
- Policy closure: `attempt.previous().len() >= 10` -> `attempt.stop()`; compare `attempt.url()` against `attempt.previous()[0]` (the ORIGINAL request URL, like Swift); allowed -> `attempt.follow()`, else `attempt.stop()` so the 3xx response is returned and `perform` turns it into `Http{30x}`.
- `send`: build the request from `HttpRequest` (header names and values through `HeaderName::from_bytes` / `HeaderValue::from_str`; a failure returns `InvalidConfiguration("invalid HTTP header")` naming only the header NAME, never the value), send, read `status().as_u16()` and `bytes().await`. Every reqwest error maps to `Network(msg)` where `msg = err.without_url().to_string()` followed by the `source()` chain joined with ": "; the message never contains the request URL.
- Cancellation is dropping the future (reqwest aborts the request); nothing to code, document it on `Transport`.

- [ ] **Step 6: Run to verify pass**

Run: `cargo test -p lingopop-core --test http_transport` then `cargo test -p lingopop-core redirect_`
Expected: PASS (12 loopback tests; 6 `redirect_` unit tests, the 7th unit test `http_timeouts_default_is_30_30_60` runs in the gate).

- [ ] **Step 7: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add reqwest transport with same-origin redirect guard and loopback tests"
```

---

### Task 3: Provider contract, registry and Claude

**Files:**
- Create: `crates/lingopop-core/src/providers/{mod.rs, claude.rs}`, `crates/lingopop-core/tests/fixtures/providers/claude/{success.json, truncated.json}`
- Modify: `crates/lingopop-core/src/lib.rs` (`pub mod providers;` and re-exports `ProviderConfig, ProviderResult, TokenUsage, AIProvider, ProviderRegistry`), `crates/lingopop-core/Cargo.toml` (`cargo add -p lingopop-core base64`), `Cargo.lock`, `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 1 `http::*`; `prompts::{system_prompt, user_content, sanitize_output, vision_prompt}`; `ProviderType::capabilities()`.
- Produces:
  - `struct ProviderConfig { pub provider_type: ProviderType, pub api_key: String, pub base_url: String, pub model: String, pub target_language: String, pub magic_prompt: String, pub referer: Option<String>, pub region: Option<String> }` (`Clone`; hand-written `Debug` printing `api_key: "<redacted>"`).
  - `struct TokenUsage { pub input_tokens: i64, pub output_tokens: i64 }` (`Debug, Clone, Copy, PartialEq, Eq`); `struct ProviderResult { pub text: String, pub usage: Option<TokenUsage>, pub truncated: bool }` (`Debug, Clone, PartialEq`).
  - `#[async_trait] trait AIProvider: Send + Sync { fn provider_type(&self) -> ProviderType; async fn execute(&self, transport: &dyn Transport, text: &str, operation: AIOperation, config: &ProviderConfig) -> Result<ProviderResult, TranslationError>; async fn execute_vision(&self, transport: &dyn Transport, image: &[u8], target_language: &str, config: &ProviderConfig) -> Result<ProviderResult, TranslationError> { /* default */ Err(OperationNotSupported { provider: self.provider_type(), operation: AIOperation::ScreenTranslate }) } }`. Capabilities are NOT part of the trait (single source: `ProviderType::capabilities()`).
  - `struct ProviderRegistry`: `standard() -> Self`, `empty() -> Self`, `insert(&mut self, Box<dyn AIProvider>)` (keyed by `provider_type()`), `get(&self, ProviderType) -> Result<&dyn AIProvider, TranslationError>` (`ProviderNotFound` when absent). In this task `standard()` registers Claude only; Tasks 4-7 add theirs and Task 7 asserts all eight.
  - Crate-private helpers in `providers/mod.rs`: `finish_text(raw: &str, sanitize: bool) -> Result<String, TranslationError>` (sanitize = `sanitize_output`, else `trim`; empty -> `EmptyResponse`), `require_api_key(&ProviderConfig) -> Result<(), TranslationError>` (empty -> `MissingApiKey`), `encode_image(image: &[u8]) -> (&'static str, String)` (mime via `http::mime_type`, `base64::engine::general_purpose::STANDARD` with padding).
  - `ClaudeProvider` (`ClaudeProvider::new()`).

**Provider test matrix** (Tasks 3-7 each name their tests `<provider>_<case>` from this list; "LLM" = Claude, OpenAI/Grok, Gemini; all via `FakeTransport`, never sockets):
- request wire: method, full URL string, header list (order-insensitive), body compared as `serde_json::Value` against an expected `json!` (`HttpRequest::header`, `json_body`).
- responses: success parsed from a fixture file under `tests/fixtures/providers/<provider>/` (loaded with `include_str!("../../tests/fixtures/providers/<provider>/<case>.json")`, hand-written after the provider docs and the Swift structs, containing real-world extra fields that must be ignored); edge cases inline with `json!`.
- cases: `translate_request_wire_format`, `vision_request_wire_format` (LLM), `success_parses_text_and_usage`, `truncated_flag`, `empty_response`, `missing_required_key_is_decoding_error`, `error_object_body_is_decoding` (HTTP 200 with `{"error":...}`), `http_error_passes_status_body_and_redacted_url`, `rejects_http_base_url`, `custom_base_with_path_trailing_slash_and_port` (base `" https://llm.example.com:8443/litellm/ "`).

- [ ] **Step 1: Write the failing tests**

`providers/mod.rs`:
```rust
// provider_config_debug_redacts_api_key        (RF 1: Debug shows "<redacted>", not "sk-ant-FIXTURE"; other fields shown)
// default_execute_vision_is_operation_not_supported (dummy provider of type DeepL -> OperationNotSupported{DeepL, ScreenTranslate}; transport saw no request)
// registry_get_missing_is_provider_not_found   (ProviderRegistry::empty().get(Claude) -> ProviderNotFound(Claude))
// registry_insert_then_get_returns_provider    (provider_type() matches)
// registry_standard_registers_claude           (replaced by standard_registry_has_all_eight in Task 7)
// finish_text_sanitizes_or_trims_and_rejects_empty ("<text>\nhi &amp; bye\n</text>": sanitize true -> "hi & bye", false -> "<text>\nhi &amp; bye\n</text>"; "  \n" both -> EmptyResponse)
// require_api_key_rejects_empty
// encode_image_uses_standard_base64_with_padding_and_sniffed_mime  ([0x89,0x50,0x4E,0x47,1] -> ("image/png", "iVBORwE="); [0xFF,0xD8,0xFF,0xE0,0] -> ("image/jpeg", "/9j/4AA="))
```
`providers/claude.rs` (`claude_*`), fixtures `success.json` (an `id`, `type: "message"`, `role`, `model`, two text blocks `"<text>\nПривіт, "` and `"світе\n</text>"`, `stop_reason: "end_turn"`, `stop_sequence: null`, `usage` with `input_tokens: 21`, `output_tokens: 7` plus `cache_creation_input_tokens`, `cache_read_input_tokens`, `service_tier`) and `truncated.json` (`stop_reason: "max_tokens"`).
```rust
// claude_translate_request_wire_format:
//   POST https://api.anthropic.com/v1/messages
//   headers {Content-Type: application/json, x-api-key: <key raw, not trimmed>, anthropic-version: 2023-06-01}, no Authorization, no anthropic-beta
//   body json!({"model": M, "max_tokens": 4096,
//               "system": system_prompt(op, target, magic),
//               "messages": [{"role":"user","content":[{"type":"text","text": user_content(text)}]}]})
// claude_vision_request_wire_format: same URL/headers; body has NO "system" key:
//   {"model":M,"max_tokens":4096,"messages":[{"role":"user","content":[
//     {"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":<std base64 padded>}},   // image FIRST
//     {"type":"text","text": vision_prompt(target)}]}]}      // raw vision prompt, not user_content
// claude_magic_operation_puts_magic_prompt_in_system     (config.magic_prompt "Do X in {{targetLanguage}}" -> system == "Do X in Ukrainian\n<fence>" via system_prompt)
// claude_text_execute_does_not_check_api_key             (empty key still sends the request: the runner validates, not the provider)
// claude_vision_without_api_key_is_missing_api_key_and_sends_nothing
// claude_success_parses_text_and_usage                   (fixture: blocks concatenated with NO separator, fence stripped -> "Привіт, світе"; usage {21, 7}; truncated false)
// claude_truncated_flag                                  (stop_reason "max_tokens" -> truncated true)
// claude_concatenates_blocks_with_text_and_skips_blocks_without                (inline: [{"type":"text","text":"a"},{"type":"tool_use"},{"type":"text","text":"b"}] -> "ab")
// claude_text_op_sanitizes_output_but_vision_only_trims  (raw "<text>x &amp; y</text>": execute -> "x & y"; execute_vision -> "<text>x &amp; y</text>")
// claude_empty_response                                  (content [] or text "  " -> EmptyResponse)
// claude_missing_required_key_is_decoding_error          (no "content"; a block without "type"; usage without "output_tokens" -> Decoding(_))
// claude_error_object_body_is_decoding                   (RF 5: 200 + {"type":"error","error":{"type":"overloaded_error","message":"x"}} -> Decoding)
// claude_http_error_passes_status_body_and_redacted_url  (429 body "slow down" -> Http{429,"slow down","https://api.anthropic.com/v1/messages"}; Display/Debug of the error lack the key)
// claude_rejects_http_base_url                           (InvalidConfiguration("baseURL must use HTTPS scheme"), no request sent)
// claude_custom_base_with_path_trailing_slash_and_port   (-> https://llm.example.com:8443/litellm/v1/messages)
// claude_user_text_with_markup_is_escaped_in_body        (RF 2: text "a & <b> 😀\n+%" -> body text block "<text>\na &amp; &lt;b&gt; 😀\n+%\n</text>"; the JSON body bytes contain the raw emoji, not \u escapes)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo add -p lingopop-core base64`, then `cargo test -p lingopop-core providers::`
Expected: FAIL to compile (module missing). `Cargo.lock` updated by `cargo add` (no `--locked`).

- [ ] **Step 3: Implement `providers/mod.rs` and `claude.rs`**

`ClaudeProvider::execute` / `execute_vision` share one private `send(transport, config, system: Option<String>, content: Vec<Block>, sanitize: bool)`: URL = `endpoint(&config.base_url, caps.default_base_url, "/v1/messages", &[])`, `json_post` with `x-api-key: config.api_key` and `anthropic-version: 2023-06-01`, `perform`, then `decode::<ResponseBody>`. Request structs derive `Serialize` with `skip_serializing_if = "Option::is_none"` on `system`; response structs follow the Swift ones exactly: `content: Vec<Block>` required, `Block { r#type: String (required), text: Option<String> }`, `stop_reason: Option<String>`, `usage: Option<{ input_tokens: i64, output_tokens: i64 }>` both required when present, unknown fields ignored. Text = `finish_text(blocks.filter_map(text).collect::<String>(), sanitize)`; `truncated = stop_reason == Some("max_tokens")`. `execute_vision` order: `require_api_key`, then endpoint, then request. Operation -> system prompt through `prompts::system_prompt(operation, &config.target_language, &config.magic_prompt)`; never re-type prompt text. Register `ClaudeProvider` in `ProviderRegistry::standard()`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core providers::`
Expected: PASS (about 26 tests).

- [ ] **Step 5: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add provider contract, registry and Claude provider"
```

---

### Task 4: OpenAI and Grok

**Files:**
- Create: `crates/lingopop-core/src/providers/openai.rs`, `crates/lingopop-core/tests/fixtures/providers/openai/{success.json, truncated.json}`, `crates/lingopop-core/tests/fixtures/providers/grok/success.json`
- Modify: `crates/lingopop-core/src/providers/mod.rs` (`mod openai;`, registry registers `OpenAiCompatible::openai()` and `OpenAiCompatible::grok()`), `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 3 `AIProvider`, `ProviderConfig`, `finish_text`, `require_api_key`, `encode_image`.
- Produces: `struct OpenAiCompatible { provider_type: ProviderType }` with `openai()` and `grok()` constructors; the default base URL comes from `provider_type.capabilities()` (`https://api.openai.com`, `https://api.x.ai`).

- [ ] **Step 1: Write the failing tests** (`openai_*` and `grok_*`; matrix from Task 3)

```rust
// openai_translate_request_wire_format:
//   POST https://api.openai.com/v1/chat/completions   (no dedup: a base ending in /v1 gives /v1/v1/chat/completions)
//   headers {Content-Type: application/json, Authorization: "Bearer <key>"}; nothing else
//   body json!({"model": M, "messages": [{"role":"system","content": system_prompt(..)},
//                                        {"role":"user","content": user_content(text)}]})   // ONLY model + messages; content is a plain string; no max_tokens, temperature, stream, response_format
// openai_vision_request_wire_format: no system message;
//   {"model":M,"messages":[{"role":"user","content":[
//     {"type":"image_url","image_url":{"url":"data:image/jpeg;base64,<b64>"}},      // image FIRST, no "detail"
//     {"type":"text","text": vision_prompt(target)}]}]}
// openai_text_execute_does_not_check_api_key; openai_vision_without_api_key_is_missing_api_key_and_sends_nothing
// openai_success_parses_text_and_usage     (fixture: id, object, created, model, choices[0] with index, message{role, content "<text>\nПривіт\n</text>", refusal null, annotations []}, finish_reason "stop", usage prompt_tokens 18, completion_tokens 5 plus *_details objects -> text "Привіт", usage {18, 5}, truncated false)
// openai_truncated_flag                    (finish_reason "length" -> true)
// openai_only_first_choice_is_used         (two choices -> text of choices[0])
// openai_null_content_is_empty_response    (RF 5: message.content null -> EmptyResponse, NOT Decoding)
// openai_empty_choices_is_empty_response   (RF 5: choices [] -> EmptyResponse)
// openai_blank_content_is_empty_response   ("  \n")
// openai_array_content_is_decoding_error   (content as [{"type":"text",...}] -> Decoding)
// openai_missing_required_key_is_decoding_error   (no "choices"; a choice without "message"; usage without completion_tokens)
// openai_error_object_body_is_decoding     (RF 5: 200 + {"error":{"message":"x","type":"invalid_request_error"}} -> Decoding)
// openai_text_op_sanitizes_but_vision_only_trims
// openai_http_error_passes_status_body_and_redacted_url; openai_rejects_http_base_url
// openai_custom_base_with_path_trailing_slash_and_port   (-> https://llm.example.com:8443/litellm/v1/chat/completions)
// grok_uses_xai_default_base_and_same_wire_format   (POST https://api.x.ai/v1/chat/completions, Bearer key, same body shape; provider_type() == Grok; fixture grok/success.json parses)
// grok_vision_request_wire_format
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core providers::openai::`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement `OpenAiCompatible`**

One private `send` shared by text and vision: path `/v1/chat/completions`, header `Authorization: Bearer {api_key}` built with `format!`. Content is an untagged enum (`Text(String)` serializes as a JSON string, `Parts(Vec<Part>)` as an array). Response structs: `choices: Vec<Choice>` required, `Choice { message: Message (required), finish_reason: Option<String> }`, `Message { content: Option<String> }`, `usage: Option<{ prompt_tokens: i64, completion_tokens: i64 }>` both required when present. Parse: `choices.first()` and `message.content` both present, else `EmptyResponse`; then `finish_text`; `truncated = finish_reason == Some("length")`; usage `prompt_tokens -> input_tokens`, `completion_tokens -> output_tokens`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core providers::openai::`
Expected: PASS (about 22 tests).

- [ ] **Step 5: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add OpenAI-compatible provider for OpenAI and Grok"
```

---

### Task 5: Gemini

**Files:**
- Create: `crates/lingopop-core/src/providers/gemini.rs`, `crates/lingopop-core/tests/fixtures/providers/gemini/{success.json, truncated.json, blocked_prompt.json}`
- Modify: `crates/lingopop-core/src/providers/mod.rs` (`mod gemini;`, registry), `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 3 contract and helpers.
- Produces: `GeminiProvider::new()`; crate-private `fn is_valid_model(model: &str) -> bool`.

- [ ] **Step 1: Write the failing tests** (`gemini_*`)

```rust
// gemini_translate_request_wire_format:
//   POST https://generativelanguage.googleapis.com/v1beta/models/<model>:generateContent   (no ?key=, no alt=sse; model only in the path)
//   headers {Content-Type: application/json, x-goog-api-key: <key>}
//   body json!({"systemInstruction":{"parts":[{"text": system_prompt(..)}]},
//               "contents":[{"role":"user","parts":[{"text": user_content(text)}]}]})   // no generationConfig, no safetySettings
// gemini_vision_request_wire_format: no systemInstruction key;
//   {"contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"image/jpeg","data":<b64>}},{"text": vision_prompt(target)}]}]}   // image FIRST
// gemini_text_execute_does_not_check_api_key; gemini_vision_without_api_key_is_missing_api_key_and_sends_nothing
// gemini_vision_checks_api_key_before_model     (empty key AND bad model -> MissingApiKey)
// gemini_model_validation_table   (valid: "gemini-3.8-flash", "gemini-2.5-pro", "a", "A1._-"; invalid: "", "-x", ".x", "_x", "gem ini", "gemini/x", "a:b", "модель", "x\n"  -> is_valid_model false; the provider returns InvalidConfiguration("Gemini model name is empty or contains unsupported characters") with no request sent, for text and vision)
// gemini_success_parses_text_and_usage   (fixture: candidates[0].content{role "model", parts [{"text":"<text>\nПривіт\n</text>"}]}, finishReason "STOP", index 0, usageMetadata{promptTokenCount 12, candidatesTokenCount 4, totalTokenCount 16, thoughtsTokenCount 9}, modelVersion, responseId -> "Привіт", usage {12, 4}; thoughts ignored)
// gemini_candidates_token_count_defaults_to_zero   (usageMetadata with only promptTokenCount -> output 0)
// gemini_truncated_flag_uses_first_candidate        (candidates[0].finishReason "MAX_TOKENS" -> true; a second candidate with MAX_TOKENS only -> false)
// gemini_concatenates_all_candidates_and_parts_skipping_parts_without_text   (two candidates x two parts incl. a part with only inlineData -> joined with no separator)
// gemini_blocked_prompt_without_candidates_is_decoding   (RF 5: fixture blocked_prompt.json = {"promptFeedback":{"blockReason":"SAFETY"},"usageMetadata":{...}} -> Decoding, NOT EmptyResponse)
// gemini_candidate_without_content_is_empty_response     (RF 5: candidates [{"finishReason":"SAFETY"}] -> EmptyResponse; candidates [] -> EmptyResponse)
// gemini_missing_prompt_token_count_is_decoding_error    (usageMetadata {} -> Decoding)
// gemini_error_object_body_is_decoding   (RF 5: 200 + {"error":{"code":400,"message":"x","status":"INVALID_ARGUMENT"}} -> Decoding)
// gemini_text_op_sanitizes_but_vision_only_trims
// gemini_http_error_passes_status_body_and_redacted_url   (400 -> url https://generativelanguage.googleapis.com/v1beta/models/<model>:generateContent)
// gemini_rejects_http_base_url
// gemini_custom_base_with_path_trailing_slash_and_port    (-> https://llm.example.com:8443/litellm/v1beta/models/<model>:generateContent)
// gemini_user_text_with_markup_is_escaped_in_body         (RF 2)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core providers::gemini::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `GeminiProvider`**

Model check is hand-written (the Swift regex is `^[A-Za-z0-9][A-Za-z0-9._-]*$`; no `regex` crate; Rust's stricter handling of a trailing newline is deliberate):
```rust
fn is_valid_model(model: &str) -> bool {
    let mut chars = model.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}
```
Order: vision = `require_api_key`, model check, endpoint; text = model check, endpoint. Path `format!("/v1beta/models/{}:generateContent", config.model)`. Request structs use `#[serde(rename_all = "camelCase")]` (`systemInstruction`, `inlineData`, `mimeType`), `systemInstruction` skipped when `None`. Response: `candidates: Vec<Candidate>` REQUIRED (no `#[serde(default)]`), `Candidate { content: Option<Content>, finish_reason: Option<String> }`, `Content { parts: Option<Vec<Part>> }`, `Part { text: Option<String> }`, `usage_metadata: Option<{ prompt_token_count: i64, candidates_token_count: Option<i64> }>`. Text = every candidate's parts' texts concatenated in order, then `finish_text`; `truncated` = first candidate `finish_reason == Some("MAX_TOKENS")`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core providers::gemini::`
Expected: PASS (about 22 tests).

- [ ] **Step 5: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add Gemini provider"
```

---

### Task 6: Google Free and Google Cloud

**Files:**
- Create: `crates/lingopop-core/src/providers/{google_free.rs, google_cloud.rs}`, fixtures `tests/fixtures/providers/google_free/success.json`, `tests/fixtures/providers/google_cloud/success.json`
- Modify: `crates/lingopop-core/src/providers/mod.rs` (`mod google_free; mod google_cloud;`, registry), `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 3 contract; Task 1 `HttpRequest::get`, `endpoint` query support; `languages::code`.
- Produces: `GoogleFreeProvider::new()`, `GoogleCloudProvider::new()`. Both ignore `operation` (Screen Translate = Translate on the OCR text); unsupported operations are rejected by the runner (Task 8), not here. `usage` is always `None`, `truncated` false.

- [ ] **Step 1: Write the failing tests**

`google_free_*` (fixture `success.json`: `[[["Доброго ранку, ","Good morning, ",null,null,10],["мій друже.","my friend.",null,null,10]],null,"en",null,null,null,null,[]]`):
```rust
// google_free_translate_request_wire_format:
//   GET https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl=uk&dt=t&q=<text>   (this query order; tl = languages::code("Ukrainian"))
//   headers exactly [("User-Agent","Mozilla/5.0")]; no body; no key
// google_free_encodes_special_characters_in_query   (RF 2: text "Привіт 😀 a&b=c+d 100% <x>\nline2" -> url.query_pairs() round-trips q to the exact text; the raw query contains "%2B" for "+", "%26" for "&", "%25" for "%", "%0A" for the newline, no raw "+" other than space encoding; ruling: Swift sends "+" raw and Google reads it as a space, this port deliberately fixes it)
// google_free_success_joins_segments_and_trims   (fixture -> "Доброго ранку, мій друже."; usage None; truncated false)
// google_free_skips_segments_without_string_first_element   ([[[null,"x"],["ok","y"]]] -> "ok")
// google_free_trims_before_the_empty_check   ([[["  \n","x"]]] -> EmptyResponse; quirk kept: Google Free trims first)
// google_free_empty_response                 ([[]] -> EmptyResponse)
// google_free_invalid_json_is_decoding_invalid_json        (body "<html>" -> Decoding("Invalid JSON"))
// google_free_unexpected_shape_is_decoding   ({"a":1}, [] and ["x"] -> Decoding("Unexpected response shape"); RF 5)
// google_free_needs_no_api_key               (empty key still sends the request)
// google_free_http_error_url_has_no_query    (429 -> Http{429, body, "https://translate.googleapis.com/translate_a/single"}; the user text appears nowhere in the error)
// google_free_rejects_http_base_url; google_free_custom_base_with_path_trailing_slash_and_port   (-> https://llm.example.com:8443/litellm/translate_a/single?...)
```
`google_cloud_*` (fixture `success.json`: `{"data":{"translations":[{"translatedText":"Доброго ранку, мій друже.","detectedSourceLanguage":"en","model":"nmt"}]}}`):
```rust
// google_cloud_translate_request_wire_format:
//   POST https://translation.googleapis.com/language/translate/v2     (no ?key=)
//   headers {Content-Type: application/json, X-goog-api-key: <key>}; "Referer" present only when config.referer is Some and non-empty
//   body json!({"q": text, "target": "uk", "model": "nmt", "format": "text"})
// google_cloud_omits_model_when_empty        (config.model "" -> body has no "model" key)
// google_cloud_sends_referer_only_when_set   (None and Some("") -> no Referer header; Some("https://app.example.com") -> header)
// google_cloud_success_parses_text           (fixture -> "Доброго ранку, мій друже."; usage None)
// google_cloud_empty_response                (translations [] or translatedText "" -> EmptyResponse; quirk kept: emptiness checked BEFORE trimming, so "  " is returned as "")
// google_cloud_missing_required_key_is_decoding_error   ({"data":{}} and {} -> Decoding)
// google_cloud_error_object_body_is_decoding   (RF 5: 200 + {"error":{"code":403,"message":"x"}} -> Decoding)
// google_cloud_http_error_passes_status_body_and_redacted_url; google_cloud_rejects_http_base_url
// google_cloud_custom_base_with_path_trailing_slash_and_port   (-> https://llm.example.com:8443/litellm/language/translate/v2)
// google_cloud_error_text_never_contains_api_key   (RF 1)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core providers::google_free::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement both providers**

Google Free: `endpoint(.., "/translate_a/single", &[("client","gtx"),("sl","auto"),("tl",&code),("dt","t"),("q",text)])` (form-encoded by `endpoint`), `HttpRequest::get(url, &[("User-Agent","Mozilla/5.0")])`, parse with `serde_json::Value`: not JSON -> `Decoding("Invalid JSON")`; top level not an array or its first element not an array -> `Decoding("Unexpected response shape")`; join the first-string element of every segment that is an array; `trim`; empty -> `EmptyResponse`. Google Cloud: request struct with `model: Option<String>` skipped when `None`; headers `X-goog-api-key` then optional `Referer`; response `data.translations[0].translatedText` required string; first translation absent or empty -> `EmptyResponse`; return `trim()` of it.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core providers::google_free::` then `cargo test -p lingopop-core providers::google_cloud::`
Expected: PASS (13 and 11 tests).

- [ ] **Step 5: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add Google Free and Google Cloud providers"
```

---

### Task 7: DeepL and Microsoft Translator (registry complete)

**Files:**
- Create: `crates/lingopop-core/src/providers/{deepl.rs, microsoft.rs}`, fixtures `tests/fixtures/providers/deepl/success.json`, `tests/fixtures/providers/microsoft/success.json`
- Modify: `crates/lingopop-core/src/providers/mod.rs` (`mod deepl; mod microsoft;`, registry now holds all eight; replace the test `registry_standard_registers_claude`), `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: Task 3 contract; `languages::{deepl_code, microsoft_code}`.
- Produces: `DeepLProvider::new()`, `MicrosoftProvider::new()`; both ignore `operation`, `usage` is `None`.

- [ ] **Step 1: Write the failing tests**

`deepl_*` (fixture: `{"translations":[{"detected_source_language":"EN","text":"Доброго ранку, мій друже.","billed_characters":24}]}`):
```rust
// deepl_translate_request_wire_format:
//   POST https://api.deepl.com/v2/translate; headers {Content-Type: application/json, Authorization: "DeepL-Auth-Key <key>"}
//   body json!({"text":[text],"target_lang":"UK"})        (languages::deepl_code; Norwegian -> "NB")
// deepl_free_key_suffix_selects_free_host      (config.base_url == "" and key "abc:fx" -> https://api-free.deepl.com/v2/translate)
// deepl_explicit_base_wins_over_key_suffix     (base "https://proxy.example.com" + ":fx" key -> proxy host)
// deepl_whitespace_only_base_falls_to_pro_default   (pins the Swift quirk: the emptiness test is on the RAW base, "  " is not empty, then endpoint() trims it to the pro default even for an ":fx" key)
// deepl_success_parses_text                    (fixture; usage None; truncated false)
// deepl_empty_response                         (translations [] or text "" -> EmptyResponse; emptiness checked before trimming)
// deepl_missing_required_key_is_decoding_error ({} and {"translations":[{}]} -> Decoding)
// deepl_error_object_body_is_decoding          (RF 5: 200 + {"message":"Quota exceeded"} -> Decoding)
// deepl_http_error_passes_status_body_and_redacted_url (456 -> Http{456,..,"https://api.deepl.com/v2/translate"}; key absent from error text)
// deepl_rejects_http_base_url; deepl_custom_base_with_path_trailing_slash_and_port   (-> https://llm.example.com:8443/litellm/v2/translate)
```
`microsoft_*` (fixture: `[{"detectedLanguage":{"language":"en","score":1.0},"translations":[{"text":"Доброго ранку, мій друже.","to":"uk"}]}]`):
```rust
// microsoft_translate_request_wire_format:
//   POST https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=uk     (query order api-version, to; languages::microsoft_code; Chinese -> "zh-Hans")
//   headers {Content-Type: application/json, Ocp-Apim-Subscription-Key: <key>}; "Ocp-Apim-Subscription-Region" only when config.region is Some and non-empty
//   body json!([{"Text": text}])
// microsoft_sends_region_only_when_set         (None and Some("") -> no header; Some("westeurope") -> header)
// microsoft_success_parses_text                (fixture; usage None)
// microsoft_empty_response                     ([] , translations [] or text "" -> EmptyResponse)
// microsoft_missing_required_key_is_decoding_error   ([{"translations":[{"text":"x"}]}] (no "to") and [{}] -> Decoding)
// microsoft_error_object_body_is_decoding      (RF 5: 200 + {"error":{"code":401000,"message":"x"}} (an object, not an array) -> Decoding)
// microsoft_http_error_passes_status_body_and_redacted_url   (401 -> url without the query "?api-version=3.0&to=uk")
// microsoft_rejects_http_base_url; microsoft_custom_base_with_path_trailing_slash_and_port   (-> https://llm.example.com:8443/litellm/translate?api-version=3.0&to=uk)
// microsoft_error_text_never_contains_api_key   (RF 1)
```
`providers/mod.rs`: `standard_registry_has_all_eight` (every `ProviderType::ALL` resolves and `get(p).provider_type() == p`).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core providers::deepl::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement both providers**

DeepL base selection (tiny algorithm the tests above pin):
```rust
let effective = if config.base_url.is_empty() {
    if config.api_key.ends_with(":fx") { "https://api-free.deepl.com" } else { caps.default_base_url }
} else { config.base_url.as_str() };
```
then `endpoint(effective, caps.default_base_url, "/v2/translate", &[])`. Microsoft: `endpoint(.., "/translate", &[("api-version","3.0"),("to",&code)])`; response is `Vec<Item>` with `Item { translations: Vec<Translation> }`, `Translation { text: String, to: String }` (both required); empty top-level array or first translation missing/empty -> `EmptyResponse`. Both return `trim()` of the text.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core providers::`
Expected: PASS (the whole `providers::` tree, including `standard_registry_has_all_eight`).

- [ ] **Step 5: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add DeepL and Microsoft Translator providers, complete the registry"
```

---

### Task 8: Operation runner with fallback

**Files:**
- Create: `crates/lingopop-core/src/runner.rs`
- Modify: `crates/lingopop-core/src/lib.rs` (`pub mod runner;` and re-exports `ConfigSnapshot, OperationRunner, RunRequest, TranslationResult, should_retry_with_fallback`), `AGENTS.md` (Layout)

**Interfaces:**
- Consumes: `ProviderRegistry`, `AIProvider`, `ProviderConfig`, `TokenUsage`, `Transport`, `Settings` (`fallback`, `magic_prompt`), `sanitize_magic_prompt`, `ProviderSettingsState`, `SecretStore::api_key`, `default_providers`, `FallbackInfo::new(primary, &err)`, `HistoryEntry::new`, `UsageRecord::new`, `Timestamp`.
- Produces (all `pub`):
  - `struct ConfigSnapshot { pub magic_prompt: String, pub fallbacks: BTreeMap<ProviderType, ProviderType>, pub connections: BTreeMap<ProviderType, ProviderConnectionSettings>, pub api_keys: BTreeMap<ProviderType, String> }` (`Clone`; hand-written `Debug` listing which providers have a key, never the keys). `from_parts(settings: &Settings, provider_settings: &ProviderSettingsState, secrets: &SecretStore) -> Self` (connections = `Loaded` entries laid over `default_providers()`; `Absent` / `Corrupt` -> `default_providers()`; `magic_prompt = sanitize_magic_prompt(&settings.magic_prompt)`; keys are copied so no lock is held across `.await`); `config(&self, provider: ProviderType, target_language: &str) -> ProviderConfig` (missing connection -> `base_url`/`model` empty like Swift; missing key -> `""`; referer/region from the connection); `fallback(&self, provider: ProviderType) -> Option<ProviderType>`.
  - `struct RunRequest { pub operation: AIOperation, pub text: String, pub detected_source: Option<String>, pub target_language: String, pub primary: ProviderType, pub allow_fallback: bool }`.
  - `struct TranslationResult { pub original: String, pub translated: String, pub target_language: String, pub provider: ProviderType, pub operation: AIOperation, pub model: String, pub via_fallback: bool, pub fallback_info: Option<FallbackInfo>, pub truncated: bool, pub usage: Option<TokenUsage>, pub detected_source: Option<String> }` (`Debug, Clone, PartialEq`); `usage_record(&self, now: Timestamp) -> Option<UsageRecord>` (`None` when `usage` is `None`; provider = the provider that answered, model = its configured model); `history_entry(&self, now: Timestamp) -> HistoryEntry` (`source_language = detected_source`).
  - `struct OperationRunner`: `new(registry: ProviderRegistry, transport: Arc<dyn Transport>) -> Self`; `async fn run(&self, request: RunRequest, configs: &ConfigSnapshot) -> Result<TranslationResult, TranslationError>`; `async fn run_vision(&self, provider: ProviderType, image: &[u8], original: String, target_language: String, configs: &ConfigSnapshot) -> Result<TranslationResult, TranslationError>`; `pub fn should_retry_with_fallback(error: &TranslationError) -> bool`.
  - Cancellation is dropping the returned future (document on `run`); the runner holds no state between calls.

- [ ] **Step 1: Write the failing tests**

Helpers in the test module: `snapshot(keys: &[(ProviderType, &str)], fallbacks: &[(ProviderType, ProviderType)]) -> ConfigSnapshot` (struct literal over `default_providers()`), `runner(fake: &Arc<FakeTransport>) -> OperationRunner` (`OperationRunner::new(ProviderRegistry::standard(), fake.clone())`), `req(op, primary, allow_fallback) -> RunRequest`, and a local `PlainProtector` (`SecretProtector` with identity `protect` / `unprotect`, copying the pattern of `XorProtector` in `secrets.rs` tests) for the `SecretStore` tests. Responses come from `include_str!("../tests/fixtures/providers/{claude,openai}/success.json")`; routing uses `FakeTransport::by_host(vec![("api.anthropic.com", HttpResponse::new(429, ".."))...])`, or `FakeTransport::new(|r| match r.url.host_str() { .. })` when a host must return `Err(Network(..))`. Tests keep an `Arc<FakeTransport>` and assert on `fake.requests()` after the run.
```rust
// should_retry_table                      (Http 429, 500, 503, 599 and Network -> true; Http 400, 401, 403, 404, 499, 600 and Decoding, EmptyResponse, MissingApiKey, InvalidUrl, InvalidConfiguration, OperationNotSupported, ProviderNotFound -> false)
// run_success_without_fallback            (Claude 200: result.provider Claude, via_fallback false, fallback_info None, translated/original/target_language/model/detected_source/usage/truncated mapped; exactly 1 request)
// run_429_falls_back_with_info            (Claude 429, fallback OpenAI 200: provider OpenAI, via_fallback true, fallback_info == Some(FallbackInfo::new(Claude, &Http{429,..})), model = OpenAI's configured model; 2 requests)
// run_503_falls_back                      (Claude 503 -> OpenAI)
// run_network_error_falls_back            (Claude Network("down") -> OpenAI)
// run_401_does_not_fall_back              (returns Http{401,..}; fallback host got no request)
// run_400_and_404_do_not_fall_back
// run_decoding_and_empty_response_do_not_fall_back   (Claude 200 with {} -> Decoding; with empty content -> EmptyResponse; no second request)
// run_allow_fallback_false_does_not_fall_back
// run_without_configured_fallback_returns_primary_error
// fallback_equal_to_primary_makes_no_second_request   (RF 4: snapshot.fallbacks.insert(Claude, Claude) -> primary error, 1 request)
// fallback_without_key_returns_primary_error_and_sends_nothing   (RF 4: OpenAI has no key -> Http{429} of Claude, zero requests to api.openai.com)
// fallback_not_supporting_the_operation_returns_primary_error    (RF 4: Rewrite, Claude 429, fallback DeepL -> primary error, zero requests to api.deepl.com)
// fallback_also_failing_returns_primary_error     (RF 4: Claude 429, OpenAI 500 -> Http{429}; 2 requests; no third)
// fallback_missing_provider_returns_primary_error (registry without OpenAI -> primary error)
// primary_unsupported_operation_is_not_supported_with_zero_requests   (DeepL + Rewrite -> OperationNotSupported{DeepL, Rewrite})
// primary_missing_key_is_missing_api_key_with_zero_requests          (Claude without key; never falls back even if a fallback is configured)
// primary_not_registered_is_provider_not_found    (empty registry)
// google_free_needs_no_key                        (Translate on GoogleFree with no key succeeds)
// magic_prompt_is_sanitized_and_reaches_the_system_prompt   (settings.magic_prompt "  Fix {{targetLanguage}}\u{1}  " -> request system text starts "Fix Ukrainian" with no \u{1}; Magic on Claude)
// target_language_override_reaches_the_request    (RunRequest.target_language "German" -> system prompt mentions German, not the snapshot default)
// usage_record_maps_provider_model_operation_tokens   (usage Some -> UsageRecord{provider raw value, model, operation raw value, input/output, timestamp = now}; usage None -> None)
// history_entry_maps_fields_and_detected_source       (source_language == detected_source; provider = the provider that answered, also after a fallback)
// run_vision_on_claude_success                    (image sniffed as png; operation ScreenTranslate; usage and truncated mapped; original = the passed string; result.provider Claude; no fallback even if configured)
// run_vision_on_deepl_is_operation_not_supported  (OperationNotSupported{DeepL, ScreenTranslate}; zero requests)
// run_vision_does_not_fall_back                   (Claude 429 with a configured fallback -> Http{429}, 1 request)
// run_vision_missing_key_is_missing_api_key       (the provider's own check; zero requests)
// config_snapshot_from_parts_loaded               (Loaded map with only Claude{base "https://proxy.example.com/litellm", model "m", referer Some} -> Claude overridden, other seven equal default_providers(); keys copied from a SecretStore built with PlainProtector + tempfile + ManualClock; fallbacks copied from Settings)
// config_snapshot_from_parts_absent_and_corrupt   (both -> connections == default_providers())
// config_snapshot_config_defaults                 (unknown connection -> base_url "" and model ""; missing key -> ""; target_language and magic_prompt filled)
// config_snapshot_debug_redacts_api_keys          (RF 1: Debug lacks "sk-ant-FIXTURE", still names the provider)
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p lingopop-core runner::`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement `runner.rs`**

`run` is a straight port of `Core/OperationRunner.swift`; the order is the contract:
```rust
let primary_cfg = configs.config(req.primary, &req.target_language);
self.validate(req.primary, req.operation, &primary_cfg)?;              // never falls back
let primary_err = match self.execute(req.primary, &req, &primary_cfg).await {
    Ok(r) => return Ok(self.finish(r, ..., false, None)),
    Err(e) => e,
};
if req.allow_fallback {
    if let Some(fb) = configs.fallback(req.primary).filter(|f| *f != req.primary) {
        if should_retry_with_fallback(&primary_err) {
            let fb_cfg = configs.config(fb, &req.target_language);
            if self.validate(fb, req.operation, &fb_cfg).is_ok() {
                if let Ok(r) = self.execute(fb, &req, &fb_cfg).await {
                    return Ok(self.finish(r, ..., true, Some(FallbackInfo::new(req.primary, &primary_err))));
                }
            }
        }
    }
}
Err(primary_err)          // any fallback failure (validation or execution) returns the PRIMARY error
```
`validate(provider, op, cfg)`: `registry.get` (`ProviderNotFound`), then `!provider_type.capabilities().supported_operations.contains(&op)` -> `OperationNotSupported { provider, operation }`, then `capabilities().requires_api_key && cfg.api_key.is_empty()` -> `MissingApiKey`. No chaining, no same-provider retry. `finish` builds `TranslationResult` (`model = cfg.model`). `run_vision`: `registry.get`; `!capabilities().supports_vision` -> `OperationNotSupported { provider, ScreenTranslate }`; no `validate`, no fallback; result operation `ScreenTranslate`, `detected_source: None`; sanitize is the provider's job (trim-only). The `Debug` for `ConfigSnapshot` prints `api_keys` as the list of providers that have a non-empty key.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lingopop-core runner::`
Expected: PASS (about 34 tests).

- [ ] **Step 5: Run the task gate and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh`
Expected: all pass.
```bash
git add -A && git commit -m "feat(core): add operation runner with single fallback and vision path"
```

---

### Task 9: Live smoke tests, vision fixture, docs and the pull request

**Files:**
- Create: `crates/lingopop-core/tests/live.rs`, `crates/lingopop-core/tests/fixtures/vision/hello.png`, `scripts/make-vision-fixture.swift`, `scripts/live-tests.sh`
- Modify: `AGENTS.md` (Layout; new section "Live provider checks"; Rules mention), `CONTRIBUTING.md` (section "Live provider checks"), `CHANGELOG.md`, `README.md` (status line: Phase 2, providers and runner)

**Interfaces:**
- Consumes: `ReqwestTransport::new(HttpTimeouts::default())`, `ProviderRegistry::standard()`, `OperationRunner`, `ConfigSnapshot`, `RunRequest`, `default_providers()`.
- Produces: `scripts/live-tests.sh` (owner-run); the PR.

- [ ] **Step 1: Generate the vision fixture**

Write `scripts/make-vision-fixture.swift` (AppKit / CoreGraphics, macOS only, usage `swift scripts/make-vision-fixture.swift <out.png>`): an `NSBitmapImageRep` with explicit `pixelsWide: 400, pixelsHigh: 120` (not `lockFocus`, which doubles on Retina), white fill, the text "Hello world" in near-black, system font ~48 pt, centred, written as PNG to the path argument; non-zero exit on bad arguments.
Run: `swift scripts/make-vision-fixture.swift crates/lingopop-core/tests/fixtures/vision/hello.png && file crates/lingopop-core/tests/fixtures/vision/hello.png`
Expected: `PNG image data, 400 x 120, 8-bit/color RGBA` (any colour type is fine). Open the file once to confirm the text is readable. The PNG is committed (`.coderabbit.yaml` already excludes fixtures and `*.png`).

- [ ] **Step 2: Write `tests/live.rs`**

Every test is `#[ignore = "live: needs API key, run scripts/live-tests.sh"]` and `#[tokio::test]`; real `ReqwestTransport::new(HttpTimeouts::default())` + `OperationRunner::new(ProviderRegistry::standard(), ..)`; the runner is called with `allow_fallback: false` over a `ConfigSnapshot` built from `default_providers()` plus the env overrides. Helpers (`fn env(var: &str) -> Option<String>`: unset or empty -> `None`; `fn skipped(var: &str)` prints `skipped: <VAR> not set` and the test returns `Ok`; `fn has_cyrillic(&str) -> bool` = any char in `\u{0400}..=\u{04FF}`) and the key rule: keys are read from the environment and never printed, not even on failure.

| Test | Env var(s) |
|---|---|
| `live_claude`, `live_openai`, `live_gemini`, `live_grok` | `LINGOPOP_LIVE_CLAUDE_KEY`, `LINGOPOP_LIVE_OPENAI_KEY`, `LINGOPOP_LIVE_GEMINI_KEY`, `LINGOPOP_LIVE_GROK_KEY` |
| `live_google_cloud` | `LINGOPOP_LIVE_GOOGLE_CLOUD_KEY` (+ optional `LINGOPOP_LIVE_GOOGLE_CLOUD_REFERER`) |
| `live_deepl` | `LINGOPOP_LIVE_DEEPL_KEY` |
| `live_microsoft` | `LINGOPOP_LIVE_MICROSOFT_KEY` (+ optional `LINGOPOP_LIVE_MICROSOFT_REGION`) |
| `live_google_free` | `LINGOPOP_LIVE_GOOGLE_FREE=1` |

Optional model override per provider: `LINGOPOP_LIVE_<P>_MODEL` (`CLAUDE`, `OPENAI`, `GEMINI`, `GROK`, `GOOGLE_CLOUD`), otherwise the `default_providers()` model. Assertions:
- every provider: Translate `"Good morning, my friend."` to `"Ukrainian"` -> non-empty, differs from the input, contains a Cyrillic char;
- LLM providers additionally: GrammarFix `"I has a apple."` -> non-empty; `run_vision` on `include_bytes!("fixtures/vision/hello.png")` (to `"Ukrainian"`) -> output contains a Cyrillic char.
There are 8 live tests; Step 4 proves by running that all of them are ignored by default.

- [ ] **Step 3: Write `scripts/live-tests.sh`**

Bash, `set -euo pipefail`, `cd` to the repo root, header comment listing the env vars (never echoing their values), then `exec cargo test -p lingopop-core --test live -- --ignored --test-threads=1 --nocapture`.

- [ ] **Step 4: Prove the live tests skip cleanly and never run in CI**

Run: `cargo test -p lingopop-core --test live`
Expected: `0 passed; 0 failed; 8 ignored`.
Run: `env -u LINGOPOP_LIVE_CLAUDE_KEY bash scripts/live-tests.sh 2>&1 | grep -c "skipped:"`
Expected: at least 1 (a missing variable prints `skipped: <VAR> not set` and the test passes). With no keys set at all, all 8 tests pass as skips and no request is sent.

- [ ] **Step 5: Documentation**

- `AGENTS.md`: Layout bullets for `tests/live.rs`, `fixtures/vision/hello.png`, `scripts/make-vision-fixture.swift`, `scripts/live-tests.sh`; new section "Live provider checks" (env var table above, owner runs `scripts/live-tests.sh` on a Mac before a release or after a provider change, never in CI, keys are never printed or committed, a failing live test after a provider API change means update the Swift-parity facts and the mock tests first).
- `CONTRIBUTING.md`: section "Live provider checks" with the same command and variable list.
- `CHANGELOG.md`: under `## Unreleased` add "HTTP transport, eight providers and the operation runner with fallback in `lingopop-core`; live smoke tests (not user-visible)".
- `README.md`: status line becomes "pre-alpha, Phase 2 (HTTP client, providers, operation runner)". No version bump.
- Re-read the Layout bullets added by Tasks 1-8 once; they must match the final file names.

- [ ] **Step 6: Run the final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && bash scripts/check-core-deps.sh && git status --short`
Expected: all pass; `git status` shows only the Task 9 files. Re-run the Task 2 forbidden-crate loop (`cargo tree -p lingopop-core -e normal -i <crate>` for `native-tls openssl openssl-sys aws-lc-rs aws-lc-sys rustls-native-certs rustls-platform-verifier core-foundation system-configuration`); expected: none found. Record the passing test counts from the `cargo test --workspace` summary lines for the PR body.

- [ ] **Step 7: Commit, push and open the pull request (pushing REQUIRES EXPLICIT GO-AHEAD FROM THE OWNER AT EXECUTION TIME)**

```bash
git add -A && git commit -m "test(core): add live provider smoke tests, vision fixture and docs"
git push -u origin phase2-providers
cat > /tmp/phase2-pr-body.md <<'EOF'
## Summary
- `http` module: `Transport` seam, `perform`, `endpoint` (https-only, Swift-identical URL rules), `FakeTransport`, `ReqwestTransport` (rustls + webpki roots + ring, same-origin redirects only, 30/30/60 s timeouts).
- Eight providers (Claude, OpenAI, Grok, Gemini, Google Free, Google Translate, DeepL, Microsoft Translator) with the same wire behaviour as the macOS app, tested against fixtures through `FakeTransport`.
- `OperationRunner`: validation, one fallback on 429 / 5xx / network, vision path; `ConfigSnapshot`, `TranslationResult` with usage and history mapping.
- Live smoke tests (`tests/live.rs`, all `#[ignore]`, run by the owner via `scripts/live-tests.sh`).
- Dependency note: reqwest 0.12.28 (0.13 needs the platform verifier and aws-lc-rs); system proxy only on Windows.

## Tests
- `cargo test --workspace --locked`: <N> passed, 8 ignored (live). Loopback transport tests: 12.
- `scripts/check-core-deps.sh` extended: forbids native-tls, openssl, aws-lc, rustls-native-certs, rustls-platform-verifier, system-configuration.

## Note
`main` is now PR-only (ruleset: `build-and-test` and CodeRabbit required, 1 approval); this is the first PR since the bootstrap.
EOF
gh pr create --base main --head phase2-providers --title "Phase 2: HTTP client, providers and operation runner" --body-file /tmp/phase2-pr-body.md
```
Fill `<N>` with the real count before running. The body must not contain any "Generated with Claude Code" footer. Expected: the PR URL is printed. Do not use `gh run watch`; check CI once later with `gh pr checks`. Likely Windows-only failures: a ring build without a C toolchain (not expected on `windows-latest`), CRLF in a fixture (`.gitattributes` forces LF), wiremock port races.
