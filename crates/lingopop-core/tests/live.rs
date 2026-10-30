//! Live smoke tests against the real provider APIs.
//!
//! Every test is `#[ignore]`d, so neither CI nor a plain `cargo test` runs them. The owner
//! runs them with `scripts/live-tests.sh`. A test whose environment variable is not set
//! prints `skipped: <VAR> not set` and returns without sending a request.
//!
//! API keys are read from the environment and are never printed, logged or asserted on;
//! `TranslationError` text never contains a key either, so a failure message is safe.

use std::sync::Arc;

use lingopop_core::errors::TranslationError;
use lingopop_core::http::{HttpTimeouts, ReqwestTransport};
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::{default_providers, ProviderType};
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::{ConfigSnapshot, OperationRunner, RunRequest, TranslationResult};

const INPUT: &str = "Good morning, my friend.";
const TARGET: &str = "Ukrainian";
const HELLO_PNG: &[u8] = include_bytes!("fixtures/vision/hello.png");

/// The variable's value, or `None` when it is unset or empty.
fn env(var: &str) -> Option<String> {
    let value = std::env::var(var).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn skipped(var: &str) {
    println!("skipped: {var} not set");
}

fn has_cyrillic(text: &str) -> bool {
    text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

/// What one provider needs from the environment.
struct Live {
    provider: ProviderType,
    /// Variable holding the API key (for Google Free: the opt-in flag, `1`).
    key_var: &'static str,
    /// Optional `LINGOPOP_LIVE_<P>_MODEL` override variable.
    model_var: Option<&'static str>,
    referer_var: Option<&'static str>,
    region_var: Option<&'static str>,
    /// LLM providers also run GrammarFix and the vision path.
    llm: bool,
}

impl Live {
    fn new(provider: ProviderType, key_var: &'static str) -> Self {
        Self {
            provider,
            key_var,
            model_var: None,
            referer_var: None,
            region_var: None,
            llm: false,
        }
    }

    fn llm(provider: ProviderType, key_var: &'static str, model_var: &'static str) -> Self {
        Self {
            model_var: Some(model_var),
            llm: true,
            ..Self::new(provider, key_var)
        }
    }

    /// `None` when the key variable is not set; otherwise the snapshot: the defaults plus the
    /// env overrides for this provider.
    fn snapshot(&self) -> Option<ConfigSnapshot> {
        let key = env(self.key_var)?;
        let mut connections = default_providers();
        if let Some(connection) = connections.get_mut(&self.provider) {
            if let Some(model) = self.model_var.and_then(env) {
                connection.model = model;
            }
            connection.referer = self.referer_var.and_then(env);
            connection.region = self.region_var.and_then(env);
        }
        let mut api_keys = std::collections::BTreeMap::new();
        if self.provider != ProviderType::GoogleFree {
            api_keys.insert(self.provider, key);
        }
        Some(ConfigSnapshot {
            magic_prompt: String::new(),
            fallbacks: std::collections::BTreeMap::new(),
            connections,
            api_keys,
        })
    }
}

fn runner() -> OperationRunner {
    let transport = ReqwestTransport::new(HttpTimeouts::default()).expect("build HTTP transport");
    OperationRunner::new(ProviderRegistry::standard(), Arc::new(transport))
}

fn request(operation: AIOperation, text: &str, provider: ProviderType) -> RunRequest {
    RunRequest {
        operation,
        text: text.to_owned(),
        detected_source: None,
        target_language: TARGET.to_owned(),
        primary: provider,
        allow_fallback: false,
    }
}

/// Unwraps a result; the panic text is the error's `Display`, which never contains a key.
fn expect_ok(
    what: &str,
    provider: ProviderType,
    result: Result<TranslationResult, TranslationError>,
) -> TranslationResult {
    match result {
        Ok(result) => result,
        Err(error) => panic!("{} {what} failed: {error}", provider.display_name()),
    }
}

async fn run_live(live: Live) {
    let Some(snapshot) = live.snapshot() else {
        skipped(live.key_var);
        return;
    };
    let provider = live.provider;
    let runner = runner();

    let translated = expect_ok(
        "translate",
        provider,
        runner
            .run(request(AIOperation::Translate, INPUT, provider), &snapshot)
            .await,
    );
    println!("{}: translate ok", provider.display_name());
    assert!(!translated.translated.is_empty(), "empty translation");
    assert_ne!(translated.translated, INPUT, "translation equals the input");
    assert!(
        has_cyrillic(&translated.translated),
        "translation has no Cyrillic character"
    );

    if !live.llm {
        return;
    }

    let fixed = expect_ok(
        "grammar fix",
        provider,
        runner
            .run(
                request(AIOperation::GrammarFix, "I has a apple.", provider),
                &snapshot,
            )
            .await,
    );
    println!("{}: grammar fix ok", provider.display_name());
    assert!(!fixed.translated.is_empty(), "empty grammar fix");

    let vision = expect_ok(
        "vision",
        provider,
        runner
            .run_vision(
                provider,
                HELLO_PNG,
                String::new(),
                TARGET.to_owned(),
                &snapshot,
            )
            .await,
    );
    println!("{}: vision ok", provider.display_name());
    assert!(
        has_cyrillic(&vision.translated),
        "vision output has no Cyrillic character"
    );
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_claude() {
    run_live(Live::llm(
        ProviderType::Claude,
        "LINGOPOP_LIVE_CLAUDE_KEY",
        "LINGOPOP_LIVE_CLAUDE_MODEL",
    ))
    .await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_openai() {
    run_live(Live::llm(
        ProviderType::OpenAI,
        "LINGOPOP_LIVE_OPENAI_KEY",
        "LINGOPOP_LIVE_OPENAI_MODEL",
    ))
    .await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_gemini() {
    run_live(Live::llm(
        ProviderType::Gemini,
        "LINGOPOP_LIVE_GEMINI_KEY",
        "LINGOPOP_LIVE_GEMINI_MODEL",
    ))
    .await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_grok() {
    run_live(Live::llm(
        ProviderType::Grok,
        "LINGOPOP_LIVE_GROK_KEY",
        "LINGOPOP_LIVE_GROK_MODEL",
    ))
    .await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_google_cloud() {
    run_live(Live {
        model_var: Some("LINGOPOP_LIVE_GOOGLE_CLOUD_MODEL"),
        referer_var: Some("LINGOPOP_LIVE_GOOGLE_CLOUD_REFERER"),
        ..Live::new(ProviderType::GoogleCloud, "LINGOPOP_LIVE_GOOGLE_CLOUD_KEY")
    })
    .await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_deepl() {
    run_live(Live::new(ProviderType::DeepL, "LINGOPOP_LIVE_DEEPL_KEY")).await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_microsoft() {
    run_live(Live {
        region_var: Some("LINGOPOP_LIVE_MICROSOFT_REGION"),
        ..Live::new(
            ProviderType::MicrosoftTranslator,
            "LINGOPOP_LIVE_MICROSOFT_KEY",
        )
    })
    .await;
}

#[tokio::test]
#[ignore = "live: needs API key, run scripts/live-tests.sh"]
async fn live_google_free() {
    // No key: the variable is an opt-in flag, `1` to run.
    const VAR: &str = "LINGOPOP_LIVE_GOOGLE_FREE";
    if env(VAR).as_deref() != Some("1") {
        skipped(VAR);
        return;
    }
    run_live(Live::new(ProviderType::GoogleFree, VAR)).await;
}
