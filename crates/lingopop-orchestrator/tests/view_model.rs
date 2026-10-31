use lingopop_core::errors::{FallbackInfo, TranslationError};
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_core::runner::TranslationResult;
use lingopop_orchestrator::view_model::{PopupPhase, PopupViewModel, ResultView};
use lingopop_orchestrator::UserMessageView;
use serde_json::json;

fn result(op: AIOperation, target: &str, text: &str) -> TranslationResult {
    TranslationResult {
        original: "orig".into(),
        translated: text.into(),
        target_language: target.into(),
        provider: ProviderType::GoogleFree,
        operation: op,
        model: "m".into(),
        via_fallback: false,
        fallback_info: None,
        truncated: false,
        usage: None,
        detected_source: None,
    }
}

fn view(r: &TranslationResult, retry: bool) -> ResultView {
    ResultView::from_result(r, retry)
}

#[test]
fn result_view_hides_replace_for_screen_translate() {
    assert!(!view(&result(AIOperation::ScreenTranslate, "English", "x"), false).show_replace);
    assert!(view(&result(AIOperation::Translate, "English", "x"), false).show_replace);
}

#[test]
fn result_view_hides_explain_for_explain() {
    assert!(!view(&result(AIOperation::Explain, "English", "x"), false).show_explain);
    assert!(view(&result(AIOperation::Rewrite, "English", "x"), false).show_explain);
}

#[test]
fn preview_is_first_120_chars() {
    let text: String = "я".repeat(200);
    let vm = PopupViewModel::loading(AIOperation::Translate, &text);
    match vm.phase {
        PopupPhase::Loading { preview, .. } => {
            assert_eq!(preview.chars().count(), 120);
            assert!(preview.len() > 120);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn action_row_counts_buttons_like_mac() {
    // Short text: Mixed only when more than the 2 labelled buttons exist.
    let short = result(AIOperation::Translate, "English", "hi");
    // Copy + Rewrite + Replace + Explain = 4 -> mixed; with Retry = 5 -> mixed.
    assert_eq!(view(&short, false).action_row.preferred, "mixed");
    assert_eq!(view(&short, true).action_row.preferred, "mixed");
    assert_eq!(
        view(&short, true).action_row.candidates,
        vec!["mixed", "compact"]
    );
    // Screen Translate without Retry: Copy + Rewrite + Explain = 3 -> mixed;
    // Explain result without Retry: Copy + Rewrite + Replace = 3 -> mixed.
    // Long text always prefers full.
    let long = result(AIOperation::Translate, "English", &"a".repeat(120));
    assert_eq!(
        view(&long, true).action_row.candidates,
        vec!["full", "mixed", "compact"]
    );
    // OCR: 2 buttons -> full even for short text; 3 with Retry -> mixed.
    let ocr = |retry: &[ProviderType]| match PopupViewModel::ocr("hi", &[], None, retry).phase {
        PopupPhase::Ocr { action_row, .. } => action_row.preferred,
        other => panic!("{other:?}"),
    };
    assert_eq!(ocr(&[]), "full");
    assert_eq!(ocr(&[ProviderType::Claude]), "mixed");
}

#[test]
fn quick_languages_insert_current_target_first() {
    let v = view(&result(AIOperation::Translate, "Klingon", "x"), false);
    assert_eq!(v.quick_languages[0], "Klingon");
    assert!(v.quick_languages.len() > 1);
    let v = view(&result(AIOperation::Translate, "English", "x"), false);
    assert_ne!(v.quick_languages[0], "Klingon");
    assert_eq!(
        v.quick_languages.iter().filter(|l| *l == "English").count(),
        1
    );
    assert!(view(&result(AIOperation::Rewrite, "English", "x"), false)
        .quick_languages
        .is_empty());
}

#[test]
fn view_model_json_shape() {
    let mut r = result(AIOperation::Translate, "Klingon", "hello");
    r.via_fallback = true;
    r.fallback_info = Some(FallbackInfo::new(
        ProviderType::Claude,
        &TranslationError::MissingApiKey,
    ));
    let vm = PopupViewModel::result(&r, &[ProviderType::OpenAI]);
    let quick = view(&r, true).quick_languages;
    assert_eq!(
        serde_json::to_value(&vm).unwrap(),
        json!({
            "phase": "result",
            "text": "hello",
            "operation": "translate",
            "provider": "Google (Free)",
            "providerName": "Google (Free)",
            "model": "m",
            "targetLanguage": "Klingon",
            "viaFallback": true,
            "fallback": {
                "primaryName": "Claude",
                "message": {"key": "API key is not set. Open Settings → Providers and add one.", "args": []},
                "details": []
            },
            "truncated": false,
            "showReplace": true,
            "showExplain": true,
            "quickLanguages": quick,
            "actionRow": {"preferred": "mixed", "candidates": ["mixed", "compact"]},
            "retryProviders": [{"provider": "OpenAI", "name": "OpenAI"}]
        })
    );

    let vm = PopupViewModel::error(
        UserMessageView::new("Invalid base URL.", vec![]),
        vec![UserMessageView::new("%@", vec!["x".into()])],
        &[],
    );
    assert_eq!(
        serde_json::to_value(&vm).unwrap(),
        json!({
            "phase": "error",
            "message": {"key": "Invalid base URL.", "args": []},
            "details": [{"key": "%@", "args": ["x"]}],
            "retryProviders": []
        })
    );

    let vm = PopupViewModel::loading(AIOperation::GrammarFix, "abc");
    assert_eq!(
        serde_json::to_value(&vm).unwrap(),
        json!({"phase": "loading", "operation": "grammarFix", "preview": "abc", "retryProviders": []})
    );
}
