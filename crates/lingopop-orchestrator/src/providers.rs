//! Which providers can be offered for Retry / OCR translation (port of
//! `App/OperationPresenter.swift` `retryProviders`, `visionCapableProviders`,
//! `translateProviders`). Providers come in `ProviderType::ALL` (declaration) order.

use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_core::runner::ConfigSnapshot;

/// A key counts as present when it is non-empty: the same rule as the core runner's
/// validation (`runner.rs`: `requires_api_key && config.api_key.is_empty()` = `MissingApiKey`).
fn has_key(configs: &ConfigSnapshot, p: ProviderType) -> bool {
    configs.api_keys.get(&p).is_some_and(|k| !k.is_empty())
}

fn ready(
    p: ProviderType,
    operation: AIOperation,
    configs: &ConfigSnapshot,
    registry_has: &impl Fn(ProviderType) -> bool,
) -> bool {
    let caps = p.capabilities();
    registry_has(p)
        && caps.supported_operations.contains(&operation)
        && (!caps.requires_api_key || has_key(configs, p))
}

/// Providers to offer for "Retry with": not `excluding`, registered, supporting `operation`,
/// with a key when one is required, and for Screen Translate also vision-capable.
pub fn retry_providers(
    excluding: ProviderType,
    operation: AIOperation,
    configs: &ConfigSnapshot,
    registry_has: impl Fn(ProviderType) -> bool,
) -> Vec<ProviderType> {
    ProviderType::ALL
        .into_iter()
        .filter(|&p| p != excluding && ready(p, operation, configs, &registry_has))
        .filter(|&p| operation != AIOperation::ScreenTranslate || p.capabilities().supports_vision)
        .collect()
}

/// Vision-capable providers that are usable now (key present when required), optionally
/// without `excluding`. Registration is not checked, like the mac `visionCapableProviders`
/// only consults its provider table; the caller filters with the registry if it needs to.
pub fn vision_providers(
    excluding: Option<ProviderType>,
    configs: &ConfigSnapshot,
) -> Vec<ProviderType> {
    ProviderType::ALL
        .into_iter()
        .filter(|&p| Some(p) != excluding)
        .filter(|&p| {
            let caps = p.capabilities();
            caps.supports_vision && (!caps.requires_api_key || has_key(configs, p))
        })
        .collect()
}

/// Providers that can translate recognized OCR text (ready for Screen Translate: supports the
/// operation and has its key). Mac `translateProviders` = `isProviderReady(_, .screenTranslate)`.
pub fn ocr_translate_providers(configs: &ConfigSnapshot) -> Vec<ProviderType> {
    ProviderType::ALL
        .into_iter()
        .filter(|&p| ready(p, AIOperation::ScreenTranslate, configs, &|_| true))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn snapshot(keys: &[ProviderType]) -> ConfigSnapshot {
        ConfigSnapshot {
            magic_prompt: String::new(),
            fallbacks: BTreeMap::new(),
            connections: BTreeMap::new(),
            api_keys: keys.iter().map(|&p| (p, "k".to_string())).collect(),
        }
    }

    #[test]
    fn retry_excludes_current_and_keyless_providers() {
        use ProviderType::*;
        let c = snapshot(&[Claude, OpenAI]);
        let list = retry_providers(Claude, AIOperation::Translate, &c, |_| true);
        assert_eq!(list, vec![OpenAI, GoogleFree]);
        let list = retry_providers(Claude, AIOperation::Translate, &c, |p| p != GoogleFree);
        assert_eq!(list, vec![OpenAI]);
        // An empty key is a missing key (core rule).
        let mut c = snapshot(&[Claude]);
        c.api_keys.insert(OpenAI, String::new());
        assert!(!retry_providers(Claude, AIOperation::Translate, &c, |_| true).contains(&OpenAI));
    }

    #[test]
    fn retry_for_screen_translate_needs_vision() {
        use ProviderType::*;
        let c = snapshot(&ProviderType::ALL);
        let list = retry_providers(Claude, AIOperation::ScreenTranslate, &c, |_| true);
        assert!(!list.is_empty());
        for p in &list {
            assert!(p.capabilities().supports_vision);
            assert!(p
                .capabilities()
                .supported_operations
                .contains(&AIOperation::ScreenTranslate));
        }
        assert!(!list.contains(&Claude));
        assert!(!list.contains(&DeepL));
    }

    #[test]
    fn google_free_needs_no_key() {
        let c = snapshot(&[]);
        assert!(ocr_translate_providers(&c).contains(&ProviderType::GoogleFree));
        assert!(!ocr_translate_providers(&c).contains(&ProviderType::Claude));
        assert_eq!(
            retry_providers(ProviderType::Claude, AIOperation::Translate, &c, |_| true),
            vec![ProviderType::GoogleFree]
        );
    }

    #[test]
    fn vision_providers_need_vision_and_key() {
        let c = snapshot(&[ProviderType::Claude, ProviderType::DeepL]);
        let list = vision_providers(None, &c);
        assert_eq!(list, vec![ProviderType::Claude]);
        assert!(vision_providers(Some(ProviderType::Claude), &c).is_empty());
    }
}
