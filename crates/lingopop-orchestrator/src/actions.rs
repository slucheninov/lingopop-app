//! Popup actions. Each action cancels the previous flow through `start`; blocking platform
//! work finishes under the gate even if its awaiting action has since been cancelled.
use lingopop_core::{operation::AIOperation, provider::ProviderType, runner::RunRequest};
use lingopop_platform::{replace_selection, PlatformError, ReplaceOptions, ReplaceOutcome};

use crate::{
    gate::GateError,
    messages::{gate_message, KEY_FOREGROUND_LOST},
    orchestrator::Orchestrator,
    view_model::{PopupViewModel, UserMessageView},
};

impl Orchestrator {
    pub async fn copy(&self) {
        self.start(|me, generation| async move {
            let text = {
                let st = me.state();
                if st.generation != generation {
                    return;
                }
                st.session.as_ref().and_then(|s| s.last_result.clone())
            };
            let Some(text) = text else {
                return;
            };
            let clipboard = me.deps().platform.clipboard.clone();
            // A failed Copy keeps the result on screen and adds a notice.
            match me
                .deps()
                .gate
                .run(move || clipboard.write_text(&text))
                .await
            {
                Ok(()) => me.clear_notice(generation),
                Err(e) => me.present_notice(generation, gate_message(&e)),
            }
        })
        .await;
    }

    pub async fn replace(&self) {
        self.start(|me, generation| async move {
            let (text, target) = {
                let st = me.state();
                if st.generation != generation {
                    return;
                }
                let Some(session) = st.session.as_ref() else {
                    return;
                };
                // Screen Translate results never offer Replace; after an unknown outcome
                // (see below) it is off for the rest of the session.
                if session.operation == AIOperation::ScreenTranslate || session.replace_disabled {
                    return;
                }
                let Some(text) = session.last_result.clone() else {
                    return;
                };
                let target = session.target_window;
                // The provider output is trimmed; the selection being overwritten may carry
                // leading / trailing whitespace (a line break) that must survive.
                let text = with_original_padding(&session.captured, &text);
                // The session stays until the paste is decided, so a gate failure can
                // show the result again.
                me.deps().popup.hide();
                me.deps().platform.dismiss.stop();
                (text, target)
            };
            let p = &me.deps().platform;
            let (fg, clip, keys, sleeper, log) = (
                p.foreground.clone(),
                p.clipboard.clone(),
                p.keys.clone(),
                p.sleeper.clone(),
                me.deps().log.clone(),
            );
            let outcome = me
                .deps()
                .gate
                .run(move || {
                    if let Some(target) = target {
                        replace_selection(
                            fg.as_ref(),
                            clip.as_ref(),
                            keys.as_ref(),
                            sleeper.as_ref(),
                            target,
                            &text,
                            &ReplaceOptions {
                                log,
                                ..Default::default()
                            },
                        )
                    } else {
                        clip.write_text(&text)?;
                        Ok(ReplaceOutcome::ForegroundLost)
                    }
                })
                .await;
            match outcome {
                Ok(ReplaceOutcome::Pasted) => me.end_session(generation),
                Ok(ReplaceOutcome::ForegroundLost) => {
                    me.end_session(generation);
                    me.present_error(
                        generation,
                        UserMessageView::new(KEY_FOREGROUND_LOST, vec![]),
                    );
                }
                Err(GateError::Platform(PlatformError::InputBlocked)) => {
                    me.end_session(generation);
                    me.present_error(
                        generation,
                        gate_message(&GateError::Platform(PlatformError::InputBlocked)),
                    );
                }
                // UNKNOWN outcome: the gate stopped waiting but the blocking job keeps running
                // and may still paste (or already did). Show the result again with a notice
                // but disable Replace for the rest of the session, so the text can never be
                // inserted twice.
                Err(e @ (GateError::TimedOut | GateError::Panicked)) => {
                    me.disable_replace(generation);
                    me.present_notice(generation, gate_message(&e));
                }
                // Confirmed failure: `Busy` (the job never started) or an error the job itself
                // returned (it stopped before pasting). Show the same result again (popup
                // re-shown at the last origin, dismiss watcher restarted by the show) with a
                // notice; Replace stays available.
                Err(e) => me.present_notice(generation, gate_message(&e)),
            }
        })
        .await;
    }

    /// Turns Replace off for the rest of this session if `generation` is still current.
    fn disable_replace(&self, generation: u64) {
        let mut st = self.state();
        if st.generation == generation {
            if let Some(session) = st.session.as_mut() {
                session.replace_disabled = true;
            }
        }
    }

    /// Clears the session if `generation` is still current.
    fn end_session(&self, generation: u64) {
        let mut st = self.state();
        if st.generation == generation {
            st.session = None;
        }
    }

    pub async fn retry(&self, provider: ProviderType) {
        self.start(move |me, generation| async move {
            let vision = {
                let st = me.state();
                st.session.as_ref().is_some_and(|s| {
                    s.operation == AIOperation::ScreenTranslate && s.image.is_some()
                })
            };
            if vision {
                me.vision_retry(generation, provider).await;
                return;
            }
            me.run_action(generation, None, Some(provider), None, false)
                .await;
        })
        .await;
    }

    pub async fn rewrite(&self) {
        self.start(|me, generation| async move {
            me.run_action(generation, Some(AIOperation::Rewrite), None, None, true)
                .await;
        })
        .await;
    }

    pub async fn explain(&self) {
        self.start(|me, generation| async move {
            me.run_action(generation, Some(AIOperation::Explain), None, None, true)
                .await;
        })
        .await;
    }

    pub async fn retranslate(&self, target_language: String) {
        self.start(move |me, generation| async move {
            me.run_action(
                generation,
                Some(AIOperation::Translate),
                None,
                Some(target_language),
                true,
            )
            .await;
        })
        .await;
    }

    async fn run_action(
        &self,
        generation: u64,
        operation: Option<AIOperation>,
        provider: Option<ProviderType>,
        target: Option<String>,
        allow_fallback: bool,
    ) {
        let settings = self.deps().config.settings();
        let snapshot = self.deps().config.snapshot();
        let request = {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            let Some(session) = st.session.as_mut() else {
                return;
            };
            if matches!(operation, Some(AIOperation::Rewrite | AIOperation::Explain)) {
                let Some(text) = session.last_result.clone() else {
                    return;
                };
                session.text = text;
                session.detected_source = None;
            }
            if let Some(op) = operation {
                session.operation = op;
                // Any explicit operation ends the Screen Translate session.
                session.image = None;
            }
            if let Some(lang) = target {
                session.target_language = lang;
            }
            session.primary = provider.unwrap_or(settings.active_provider);
            let request = RunRequest {
                operation: session.operation,
                text: session.text.clone(),
                detected_source: session.detected_source.clone(),
                target_language: session.target_language.clone(),
                primary: session.primary,
                allow_fallback,
            };
            self.present_locked(
                &mut st,
                &PopupViewModel::loading(request.operation, &request.text),
            );
            request
        };
        self.run_and_present(generation, &settings, &snapshot, request)
            .await;
    }

    pub async fn close(&self) {
        self.start(|me, generation| async move {
            let mut st = me.state();
            if st.generation != generation {
                return;
            }
            me.deps().popup.hide();
            me.deps().platform.dismiss.stop();
            st.session = None;
        })
        .await;
    }

    pub async fn escape(&self) {
        self.close().await;
    }
}

/// `replacement` with its own surrounding whitespace removed, wrapped in the leading and trailing
/// whitespace of `original` (the selection Replace overwrites), so a selected line keeps its
/// line break. An all-whitespace `original` is kept whole as the leading part (never doubled).
fn with_original_padding(original: &str, replacement: &str) -> String {
    let core = replacement.trim();
    let body = original.trim_start();
    let leading = &original[..original.len() - body.len()];
    let trailing = &body[body.trim_end().len()..];
    format!("{leading}{core}{trailing}")
}

#[cfg(test)]
mod padding_tests {
    use super::with_original_padding as pad;

    #[test]
    fn keeps_trailing_crlf() {
        assert_eq!(pad("Hello.\r\n", "Hallo."), "Hallo.\r\n");
    }

    #[test]
    fn keeps_leading_and_trailing_whitespace() {
        assert_eq!(pad("  \tHello \n", "Hallo"), "  \tHallo \n");
    }

    #[test]
    fn no_padding_is_unchanged() {
        assert_eq!(pad("Hello", "Hallo"), "Hallo");
    }

    #[test]
    fn replacement_whitespace_is_not_doubled() {
        assert_eq!(pad("Hello\n", " Hallo\n\n"), "Hallo\n");
        assert_eq!(pad("Hello", "\n Hallo \n"), "Hallo");
    }

    #[test]
    fn all_whitespace_original_is_kept_once() {
        assert_eq!(pad(" \n", "Hallo"), " \nHallo");
        assert_eq!(pad("", "Hallo"), "Hallo");
    }
}
