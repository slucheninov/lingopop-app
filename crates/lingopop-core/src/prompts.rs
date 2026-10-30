//! Prompt texts. English, copied verbatim from `Core/PromptBuilder.swift`; never localized.

/// The user-editable Magic prompt default (`PromptBuilder.defaultMagicPrompt`).
/// `{{targetLanguage}}` is replaced by the target language name when it is used.
pub const DEFAULT_MAGIC_PROMPT: &str = "You are a professional writing assistant. Fix all grammar, spelling, and punctuation errors, improve clarity, flow, and style, and if the text is not in {{targetLanguage}}, translate it into {{targetLanguage}}. Return only the result - no explanations, no notes.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-copied anchors from `Core/PromptBuilder.swift:4` (`defaultMagicPrompt`).
    #[test]
    fn default_magic_prompt_literal_anchor() {
        assert!(DEFAULT_MAGIC_PROMPT.starts_with(
            "You are a professional writing assistant. Fix all grammar, spelling, and punctuation errors,"
        ));
        assert!(
            DEFAULT_MAGIC_PROMPT.ends_with("Return only the result - no explanations, no notes.")
        );
        assert_eq!(
            DEFAULT_MAGIC_PROMPT.matches("{{targetLanguage}}").count(),
            2
        );
    }
}
