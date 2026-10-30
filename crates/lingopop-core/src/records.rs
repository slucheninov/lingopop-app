use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::operation::AIOperation;
use crate::provider::ProviderType;
use crate::serde_uuid;
use crate::time::Timestamp;

/// One translation history row (`history.json` item). `provider` and
/// `operation` hold the raw persisted strings so values written by a newer
/// version survive a rewrite; use the typed accessors to interpret them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    pub original: String,
    pub translated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_language: Option<String>,
    pub target_language: String,
    pub provider: String,
    pub operation: String,
    pub timestamp: Timestamp,
}

impl HistoryEntry {
    pub fn new(
        original: String,
        translated: String,
        source_language: Option<String>,
        target_language: String,
        provider: ProviderType,
        operation: AIOperation,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            original,
            translated,
            source_language,
            target_language,
            provider: provider.raw_value().to_owned(),
            operation: operation.raw_value().to_owned(),
            timestamp,
        }
    }

    pub fn provider_type(&self) -> Option<ProviderType> {
        ProviderType::from_raw_value(&self.provider)
    }

    pub fn operation_type(&self) -> Option<AIOperation> {
        AIOperation::from_raw_value(&self.operation)
    }
}

/// One token-usage record (`usage.json` item); same raw-string rule as
/// [`HistoryEntry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    #[serde(with = "serde_uuid")]
    pub id: Uuid,
    pub timestamp: Timestamp,
    pub provider: String,
    pub model: String,
    pub operation: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl UsageRecord {
    pub fn new(
        provider: ProviderType,
        model: String,
        operation: AIOperation,
        input_tokens: i64,
        output_tokens: i64,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp,
            provider: provider.raw_value().to_owned(),
            model,
            operation: operation.raw_value().to_owned(),
            input_tokens,
            output_tokens,
        }
    }

    pub fn provider_type(&self) -> Option<ProviderType> {
        ProviderType::from_raw_value(&self.provider)
    }

    pub fn operation_type(&self) -> Option<AIOperation> {
        AIOperation::from_raw_value(&self.operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AIOperation, ProviderType, Timestamp};

    const HISTORY_LINE: &str = r#"{"original":"a","targetLanguage":"German","translated":"b","operation":"translate","timestamp":815059756.035193,"provider":"Claude","id":"16990D16-7E15-47C3-A299-EF2AEC4400D1"}"#;
    const USAGE_LINE: &str = r#"{"inputTokens":1,"outputTokens":2,"timestamp":815059756.035515,"id":"432F3827-40A1-430B-A818-67B2B6A36970","model":"m","provider":"Claude","operation":"translate"}"#;

    fn sample_history() -> HistoryEntry {
        HistoryEntry::new(
            "a".into(),
            "b".into(),
            None,
            "German".into(),
            ProviderType::Claude,
            AIOperation::Translate,
            Timestamp::from_apple_seconds(815059756.035193),
        )
    }

    #[test]
    fn history_entry_decodes_real_swift_output() {
        let entry: HistoryEntry = serde_json::from_str(HISTORY_LINE).unwrap();
        assert_eq!(entry.id.to_string(), "16990d16-7e15-47c3-a299-ef2aec4400d1");
        assert_eq!(entry.provider_type(), Some(ProviderType::Claude));
        assert_eq!(entry.operation_type(), Some(AIOperation::Translate));
        assert_eq!(entry.source_language, None);
        let reencoded = serde_json::to_value(&entry).unwrap();
        let original: serde_json::Value = serde_json::from_str(HISTORY_LINE).unwrap();
        assert_eq!(reencoded, original);
    }

    #[test]
    fn history_entry_omits_source_language_when_none() {
        let mut entry = sample_history();
        let value = serde_json::to_value(&entry).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "id",
                "operation",
                "original",
                "provider",
                "targetLanguage",
                "timestamp",
                "translated"
            ]
        );
        entry.source_language = Some("English".into());
        let value = serde_json::to_value(&entry).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "id",
                "operation",
                "original",
                "provider",
                "sourceLanguage",
                "targetLanguage",
                "timestamp",
                "translated"
            ]
        );
        assert_eq!(value["sourceLanguage"], "English");
        let back: HistoryEntry = serde_json::from_value(value).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn usage_record_decodes_real_swift_output() {
        let rec: UsageRecord = serde_json::from_str(USAGE_LINE).unwrap();
        assert_eq!(rec.input_tokens, 1);
        assert_eq!(rec.output_tokens, 2);
        assert_eq!(rec.model, "m");
        assert_eq!(rec.provider_type(), Some(ProviderType::Claude));
        assert_eq!(rec.operation_type(), Some(AIOperation::Translate));
        let reencoded = serde_json::to_value(&rec).unwrap();
        let original: serde_json::Value = serde_json::from_str(USAGE_LINE).unwrap();
        assert_eq!(reencoded, original);
    }

    #[test]
    fn unknown_provider_and_operation_survive_round_trip() {
        let line = HISTORY_LINE
            .replace("\"Claude\"", "\"FutureAI\"")
            .replace("\"translate\"", "\"summarize\"");
        let entry: HistoryEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(entry.provider_type(), None);
        assert_eq!(entry.operation_type(), None);
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value["provider"], "FutureAI");
        assert_eq!(value["operation"], "summarize");

        let line = USAGE_LINE
            .replace("\"Claude\"", "\"FutureAI\"")
            .replace("\"translate\"", "\"summarize\"");
        let rec: UsageRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(rec.provider_type(), None);
        assert_eq!(rec.operation_type(), None);
        let value = serde_json::to_value(&rec).unwrap();
        assert_eq!(value["provider"], "FutureAI");
        assert_eq!(value["operation"], "summarize");
    }

    #[test]
    fn new_constructors_store_raw_values() {
        let ts = Timestamp::from_apple_seconds(1.0);
        let entry = HistoryEntry::new(
            "a".into(),
            "b".into(),
            Some("English".into()),
            "German".into(),
            ProviderType::GoogleFree,
            AIOperation::GrammarFix,
            ts,
        );
        assert_eq!(entry.provider, "Google (Free)");
        assert_eq!(entry.operation, "grammarFix");
        assert_eq!(entry.provider_type(), Some(ProviderType::GoogleFree));
        assert_eq!(entry.operation_type(), Some(AIOperation::GrammarFix));
        let other = HistoryEntry::new(
            "a".into(),
            "b".into(),
            None,
            "German".into(),
            ProviderType::Claude,
            AIOperation::Translate,
            ts,
        );
        assert_ne!(entry.id, other.id, "each entry gets a fresh v4 id");
        assert_eq!(entry.id.get_version_num(), 4);

        let rec = UsageRecord::new(
            ProviderType::MicrosoftTranslator,
            "m".into(),
            AIOperation::ScreenTranslate,
            3,
            4,
            ts,
        );
        assert_eq!(rec.provider, "Microsoft Translator");
        assert_eq!(rec.operation, "screenTranslate");
        assert_eq!((rec.input_tokens, rec.output_tokens), (3, 4));
    }

    #[test]
    fn missing_required_field_is_an_error() {
        let line = HISTORY_LINE.replace("\"translated\":\"b\",", "");
        assert!(serde_json::from_str::<HistoryEntry>(&line).is_err());
        let line = USAGE_LINE.replace("\"model\":\"m\",", "");
        assert!(serde_json::from_str::<UsageRecord>(&line).is_err());
    }

    #[test]
    fn lowercase_id_is_read_and_rewritten_uppercase() {
        let line = HISTORY_LINE.replace(
            "16990D16-7E15-47C3-A299-EF2AEC4400D1",
            "16990d16-7e15-47c3-a299-ef2aec4400d1",
        );
        let entry: HistoryEntry = serde_json::from_str(&line).unwrap();
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value["id"], "16990D16-7E15-47C3-A299-EF2AEC4400D1");
    }
}
