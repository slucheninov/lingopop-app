use serde::{Deserialize, Serialize};

/// An AI operation. Raw values are persisted identifiers (history, usage,
/// settings maps) and must never be renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AIOperation {
    Translate,
    GrammarFix,
    Rewrite,
    Explain,
    ScreenTranslate,
    Magic,
}

impl AIOperation {
    /// Every operation, in declaration (UI) order.
    pub const ALL: [AIOperation; 6] = [
        AIOperation::Translate,
        AIOperation::GrammarFix,
        AIOperation::Rewrite,
        AIOperation::Explain,
        AIOperation::ScreenTranslate,
        AIOperation::Magic,
    ];

    /// The frozen persisted identifier.
    pub fn raw_value(self) -> &'static str {
        match self {
            AIOperation::Translate => "translate",
            AIOperation::GrammarFix => "grammarFix",
            AIOperation::Rewrite => "rewrite",
            AIOperation::Explain => "explain",
            AIOperation::ScreenTranslate => "screenTranslate",
            AIOperation::Magic => "magic",
        }
    }

    /// Parses a persisted identifier; unknown values (from a newer version) give `None`.
    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.raw_value() == raw)
    }

    /// English UI name (the mac `AIOperation.displayName` key); localized later.
    pub fn english_name(self) -> &'static str {
        match self {
            AIOperation::Translate => "Translate",
            AIOperation::GrammarFix => "Fix grammar",
            AIOperation::Rewrite => "Rewrite",
            AIOperation::Explain => "Explain",
            AIOperation::ScreenTranslate => "Screen Translate",
            AIOperation::Magic => "Magic",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_raw_values_are_frozen() {
        assert_eq!(
            AIOperation::ALL.map(|o| o.raw_value()),
            [
                "translate",
                "grammarFix",
                "rewrite",
                "explain",
                "screenTranslate",
                "magic"
            ]
        );
    }

    #[test]
    fn serde_uses_raw_values() {
        for op in AIOperation::ALL {
            let json = serde_json::to_string(&op).unwrap();
            assert_eq!(json, format!("\"{}\"", op.raw_value()));
            assert_eq!(serde_json::from_str::<AIOperation>(&json).unwrap(), op);
            assert_eq!(AIOperation::from_raw_value(op.raw_value()), Some(op));
        }
    }

    #[test]
    fn unknown_raw_value_is_none_not_a_panic() {
        assert_eq!(AIOperation::from_raw_value("bogus"), None);
        assert_eq!(AIOperation::from_raw_value(""), None);
        assert_eq!(AIOperation::from_raw_value("Translate"), None);
        assert!(serde_json::from_str::<AIOperation>("\"bogus\"").is_err());
    }

    #[test]
    fn english_names_match_mac_display_names() {
        assert_eq!(
            AIOperation::ALL.map(|o| o.english_name()),
            [
                "Translate",
                "Fix grammar",
                "Rewrite",
                "Explain",
                "Screen Translate",
                "Magic"
            ]
        );
    }

    #[test]
    fn ordering_follows_declaration_order() {
        let mut sorted = AIOperation::ALL;
        sorted.sort();
        assert_eq!(sorted, AIOperation::ALL);
    }
}
