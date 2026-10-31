//! The popup view model: the JSON the frontend renders. camelCase fields; `AIOperation` and
//! `ProviderType` serialize as their frozen raw values (core's serde names); action-row modes
//! as `"full" | "mixed" | "compact"`; texts as `UserMessageView` catalog keys + args.

use lingopop_core::action_row::{ActionRowMode, LONG_TEXT_CHARACTER_THRESHOLD};
use lingopop_core::errors::{FallbackInfo, UserMessage};
use lingopop_core::languages;
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_core::runner::TranslationResult;
use serde::Serialize;

/// Characters of the original shown while loading.
pub const PREVIEW_CHARS: usize = 120;

/// A catalog key and its `%@` arguments (see `lingopop_core::errors::UserMessage`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserMessageView {
    pub key: String,
    pub args: Vec<String>,
}

impl UserMessageView {
    pub fn new(key: &str, args: Vec<String>) -> Self {
        Self {
            key: key.to_string(),
            args,
        }
    }
}

impl From<UserMessage> for UserMessageView {
    fn from(m: UserMessage) -> Self {
        Self {
            key: m.key.to_string(),
            args: m.args,
        }
    }
}

/// A provider entry of a menu: `provider` is the raw value, `name` the display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetryProvider {
    pub provider: ProviderType,
    pub name: String,
}

impl RetryProvider {
    pub fn new(provider: ProviderType) -> Self {
        Self {
            provider,
            name: provider.display_name().to_string(),
        }
    }

    pub fn list(providers: &[ProviderType]) -> Vec<Self> {
        providers.iter().copied().map(Self::new).collect()
    }
}

fn mode_str(m: ActionRowMode) -> &'static str {
    match m {
        ActionRowMode::Full => "full",
        ActionRowMode::Mixed => "mixed",
        ActionRowMode::Compact => "compact",
    }
}

/// Which action-row modes the frontend tries, widest-preferred first (`candidates[0] == preferred`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionRowView {
    pub preferred: &'static str,
    pub candidates: Vec<&'static str>,
}

impl ActionRowView {
    /// `char_count` is the length of the shown text, `button_count` the buttons of the row.
    pub fn new(char_count: usize, button_count: usize) -> Self {
        let preferred =
            ActionRowMode::preferred(char_count, LONG_TEXT_CHARACTER_THRESHOLD, button_count);
        Self {
            preferred: mode_str(preferred),
            candidates: preferred
                .candidates()
                .iter()
                .map(|&m| mode_str(m))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackView {
    pub primary_name: String,
    pub message: UserMessageView,
    pub details: Vec<UserMessageView>,
}

impl From<&FallbackInfo> for FallbackView {
    fn from(f: &FallbackInfo) -> Self {
        Self {
            primary_name: f.primary_provider.display_name().to_string(),
            message: f.message.clone().into(),
            details: f
                .details
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultView {
    pub text: String,
    pub operation: AIOperation,
    pub provider: ProviderType,
    pub provider_name: String,
    pub model: String,
    pub target_language: String,
    pub via_fallback: bool,
    pub fallback: Option<FallbackView>,
    pub truncated: bool,
    pub show_replace: bool,
    pub show_explain: bool,
    pub quick_languages: Vec<String>,
    pub action_row: ActionRowView,
}

impl ResultView {
    /// `retry_available`: the Retry menu has at least one provider (counts as a button).
    pub fn from_result(r: &TranslationResult, retry_available: bool) -> Self {
        let show_replace = r.operation != AIOperation::ScreenTranslate;
        let show_explain = r.operation != AIOperation::Explain;
        let quick_languages = if r.operation == AIOperation::Translate {
            let mut list: Vec<String> = languages::quick_list()
                .into_iter()
                .map(String::from)
                .collect();
            if !list.contains(&r.target_language) {
                list.insert(0, r.target_language.clone());
            }
            list
        } else {
            Vec::new()
        };
        // Copy + Rewrite, then the optional buttons (mac popup action row).
        let button_count = 2
            + usize::from(show_replace)
            + usize::from(show_explain)
            + usize::from(retry_available);
        Self {
            text: r.translated.clone(),
            operation: r.operation,
            provider: r.provider,
            provider_name: r.provider.display_name().to_string(),
            model: r.model.clone(),
            target_language: r.target_language.clone(),
            via_fallback: r.via_fallback,
            fallback: r.fallback_info.as_ref().map(FallbackView::from),
            truncated: r.truncated,
            show_replace,
            show_explain,
            quick_languages,
            action_row: ActionRowView::new(r.translated.chars().count(), button_count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorView {
    pub message: UserMessageView,
    pub details: Vec<UserMessageView>,
}

/// Internally tagged by `"phase"` (`loading` / `ocr` / `result` / `error`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PopupPhase {
    Loading {
        operation: AIOperation,
        preview: String,
    },
    Ocr {
        text: String,
        translate_providers: Vec<RetryProvider>,
        active_translate_provider: Option<ProviderType>,
        action_row: ActionRowView,
    },
    Result(ResultView),
    Error(ErrorView),
}

/// What the popup renders. `phase` is flattened, so the JSON is
/// `{"phase": "result", ..., "retryProviders": [...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupViewModel {
    #[serde(flatten)]
    pub phase: PopupPhase,
    pub retry_providers: Vec<RetryProvider>,
}

impl PopupViewModel {
    pub fn loading(operation: AIOperation, original: &str) -> Self {
        Self {
            phase: PopupPhase::Loading {
                operation,
                preview: original.chars().take(PREVIEW_CHARS).collect(),
            },
            retry_providers: Vec::new(),
        }
    }

    pub fn result(result: &TranslationResult, retry_providers: &[ProviderType]) -> Self {
        Self {
            phase: PopupPhase::Result(ResultView::from_result(result, !retry_providers.is_empty())),
            retry_providers: RetryProvider::list(retry_providers),
        }
    }

    pub fn error(
        message: UserMessageView,
        details: Vec<UserMessageView>,
        retry_providers: &[ProviderType],
    ) -> Self {
        Self {
            phase: PopupPhase::Error(ErrorView { message, details }),
            retry_providers: RetryProvider::list(retry_providers),
        }
    }

    /// The OCR phase: recognized text with a provider picker for translating it and the Retry
    /// menu (send the image to AI). Row = Copy + Translate, plus Retry when available.
    pub fn ocr(
        text: &str,
        translate_providers: &[ProviderType],
        active: Option<ProviderType>,
        retry_providers: &[ProviderType],
    ) -> Self {
        let button_count = 2 + usize::from(!retry_providers.is_empty());
        Self {
            phase: PopupPhase::Ocr {
                text: text.to_string(),
                translate_providers: RetryProvider::list(translate_providers),
                active_translate_provider: active,
                action_row: ActionRowView::new(text.chars().count(), button_count),
            },
            retry_providers: RetryProvider::list(retry_providers),
        }
    }
}
