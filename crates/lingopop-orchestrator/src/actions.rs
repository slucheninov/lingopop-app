//! Popup actions. Each action cancels the previous flow through `start`; blocking platform
//! work finishes under the gate even if its awaiting action has since been cancelled.
use lingopop_core::{operation::AIOperation, provider::ProviderType, runner::RunRequest};
use lingopop_platform::{replace_selection, ReplaceOptions, ReplaceOutcome};

use crate::{
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
            if let Err(e) = me
                .deps()
                .gate
                .run(move || clipboard.write_text(&text))
                .await
            {
                me.present_error(generation, gate_message(&e));
            }
        })
        .await;
    }

    pub async fn replace(&self) {
        self.start(|me, generation| async move {
            let (text, target) = {
                let mut st = me.state();
                if st.generation != generation {
                    return;
                }
                let Some(session) = st.session.as_ref() else {
                    return;
                };
                let Some(text) = session.last_result.clone() else {
                    return;
                };
                let target = session.target_window;
                me.deps().popup.hide();
                st.session = None;
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
                Ok(ReplaceOutcome::Pasted) => {}
                Ok(ReplaceOutcome::ForegroundLost) => me.present_error(
                    generation,
                    UserMessageView::new(KEY_FOREGROUND_LOST, vec![]),
                ),
                Err(e) => me.present_error(generation, gate_message(&e)),
            }
        })
        .await;
    }

    pub async fn retry(&self, provider: ProviderType) {
        self.start(move |me, generation| async move {
            let screen_translate = {
                let st = me.state();
                st.session
                    .as_ref()
                    .is_some_and(|s| s.operation == AIOperation::ScreenTranslate)
            };
            if screen_translate {
                // Task 5: use vision retry when the session has a captured image.
                // Currently ScreenTranslate uses the text path.
                me.run_action(generation, None, Some(provider), None, false)
                    .await;
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
            st.session = None;
        })
        .await;
    }

    pub async fn escape(&self) {
        self.close().await;
    }
}
