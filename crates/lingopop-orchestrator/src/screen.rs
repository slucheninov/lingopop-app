//! The Screen Translate flow (port of `App/ScreenTranslateFlow.swift`): region pick, capture of
//! the clipped region, OCR, translating the recognized text and the vision retry.

use async_trait::async_trait;
use lingopop_platform::{PlatformError, Rect};

/// The interactive region picker (a full-screen overlay in the shell). It is async UI, not a
/// blocking platform call, so it does not go through the gate. Dropping the future cancels it.
#[async_trait]
pub trait RegionPicker: Send + Sync {
    /// The selected rectangle in physical virtual-screen pixels; `Ok(None)` = the user cancelled.
    async fn pick(&self) -> Result<Option<Rect>, PlatformError>;
}

use std::sync::Arc;

use lingopop_core::detector;
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_core::runner::RunRequest;
use lingopop_core::target_language::resolve_target_language;
use lingopop_platform::{validate_region, MonitorInfo, Point, WindowHandle};

use crate::messages::{gate_message, no_text_found, platform_message};
use crate::orchestrator::Orchestrator;
use crate::providers::{ocr_translate_providers, vision_providers};
use crate::session::Session;
use crate::view_model::{PopupViewModel, UserMessageView};

/// What the first gate job reads: cursor and monitors for placing an early error popup, and
/// the window that was in the foreground before the picker.
struct Prelude {
    cursor: Option<Point>,
    monitors: Option<Vec<MonitorInfo>>,
    target: Option<WindowHandle>,
}

impl Orchestrator {
    /// Translates the recognized OCR text with `provider` (the OCR phase's Translate button).
    /// Fallback is allowed only for the active provider. Does nothing without OCR text in the
    /// Screen Translate session.
    pub async fn translate_ocr(&self, provider: ProviderType) {
        self.start(move |me, generation| async move {
            let settings = me.deps().config.settings();
            let snapshot = me.deps().config.snapshot();
            let request = {
                let mut st = me.state();
                if st.generation != generation {
                    return;
                }
                let Some(session) = st.session.as_mut() else {
                    return;
                };
                if session.operation != AIOperation::ScreenTranslate
                    || session.text.trim().is_empty()
                {
                    return;
                }
                session.primary = provider;
                let request = RunRequest {
                    operation: AIOperation::ScreenTranslate,
                    text: session.text.clone(),
                    detected_source: session.detected_source.clone(),
                    target_language: session.target_language.clone(),
                    primary: provider,
                    allow_fallback: provider == settings.active_provider,
                };
                me.present_locked(
                    &mut st,
                    &PopupViewModel::loading(request.operation, &request.text),
                );
                request
            };
            me.run_and_present(generation, &settings, &snapshot, request)
                .await;
        })
        .await;
    }

    fn present_error_with_vision(&self, generation: u64, message: UserMessageView) {
        let snapshot = self.deps().config.snapshot();
        let has = |p: ProviderType| (self.deps().registry_has)(p);
        let vision = vision_providers(None, &snapshot, has);
        self.present(
            generation,
            &PopupViewModel::error(message, Vec::new(), &vision),
        );
    }

    pub(crate) async fn screen_flow(&self, generation: u64) {
        let d = self.deps();
        // 1. Target window, cursor and monitors, before the picker overlay appears.
        let (foreground, monitors) = (d.platform.foreground.clone(), d.platform.monitors.clone());
        let prelude = d
            .gate
            .run(move || {
                Ok(Prelude {
                    cursor: monitors.cursor_position().ok(),
                    monitors: monitors.monitors().ok(),
                    target: foreground.current(),
                })
            })
            .await;
        if !self.is_current(generation) {
            return;
        }
        let prelude = match prelude {
            Ok(p) => p,
            Err(e) => {
                self.log("screen translate: platform did not run");
                self.present_error(generation, gate_message(&e));
                return;
            }
        };
        {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            if let Some(c) = prelude.cursor {
                st.cursor = c;
            }
            if let Some(m) = prelude.monitors {
                st.monitors = m;
            }
        }

        // 2. The region (async UI, cancelled by dropping this future).
        let region = match d.picker.pick().await {
            Ok(Some(region)) => region,
            Ok(None) => return,
            Err(e) => {
                self.log("screen translate: region picker failed");
                self.present_error(generation, platform_message(&e));
                return;
            }
        };

        // 3. One gate job: validate (clip) and capture the clipped rect.
        let (monitors, capture) = (d.platform.monitors.clone(), d.platform.capture.clone());
        let captured = d
            .gate
            .run(move || {
                let list = monitors.monitors()?;
                let clipped = validate_region(region, &list)?;
                let image = capture.capture_region(clipped)?;
                Ok((list, clipped, image))
            })
            .await;
        if !self.is_current(generation) {
            return;
        }
        let (monitor_list, clipped, image) = match captured {
            Ok(c) => c,
            Err(e) => {
                self.log("screen translate: capture failed");
                self.present_error(generation, gate_message(&e));
                return;
            }
        };
        let settings = d.config.settings();
        let image = Arc::new(image);
        // 4. The session (the image is kept for the vision retry) and the loading popup at the
        // bottom-right of the clipped region.
        {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            st.cursor = Point {
                x: clipped.right(),
                y: clipped.bottom(),
            };
            st.monitors = monitor_list;
            st.session = Some(Session {
                operation: AIOperation::ScreenTranslate,
                text: String::new(),
                last_result: None,
                detected_source: None,
                target_language: settings.target_language.clone(),
                target_window: prelude.target,
                primary: settings.active_provider,
                image: Some(image.clone()),
            });
            self.present_locked(
                &mut st,
                &PopupViewModel::loading(AIOperation::ScreenTranslate, ""),
            );
        }

        // 5. OCR.
        let ocr = d.platform.ocr.clone();
        let recognized = d.gate.run(move || ocr.recognize(&image)).await;
        if !self.is_current(generation) {
            return;
        }
        match recognized {
            Ok(ocr_text) if !ocr_text.is_empty() => {
                let text = ocr_text.text.trim().to_string();
                self.log(&format!("ocr: {} characters", text.chars().count()));
                let detected = detector::detect(&text).map(str::to_string);
                let target = resolve_target_language(&settings, detected.as_deref());
                let snapshot = d.config.snapshot();
                let has = |p: ProviderType| (d.registry_has)(p);
                let translate = ocr_translate_providers(&snapshot, has);
                let vision = vision_providers(None, &snapshot, has);
                // The active provider is preselected only when it can translate right now.
                let active = translate
                    .contains(&settings.active_provider)
                    .then_some(settings.active_provider);
                {
                    let mut st = self.state();
                    if st.generation != generation {
                        return;
                    }
                    if let Some(session) = st.session.as_mut() {
                        session.text = text.clone();
                        session.detected_source = detected;
                        session.target_language = target;
                    }
                    self.present_locked(
                        &mut st,
                        &PopupViewModel::ocr(&text, &translate, active, &vision),
                    );
                }
                if settings.auto_copy_ocr {
                    self.auto_copy(generation, text).await;
                }
            }
            Ok(_) => {
                self.log("ocr: no text");
                let snapshot = d.config.snapshot();
                let has = |p: ProviderType| (d.registry_has)(p);
                let any_vision = !vision_providers(None, &snapshot, has).is_empty();
                self.present_error_with_vision(generation, no_text_found(any_vision));
            }
            Err(e) => {
                self.log("ocr failed");
                self.present_error_with_vision(generation, gate_message(&e));
            }
        }
    }

    /// Sends the captured image to a vision provider (OCR and translation in one request). No
    /// fallback; the original is the OCR text when there is one.
    pub(crate) async fn vision_retry(&self, generation: u64, provider: ProviderType) {
        let d = self.deps();
        let settings = d.config.settings();
        let snapshot = d.config.snapshot();
        let (image, original, target) = {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            let Some(session) = st.session.as_mut() else {
                return;
            };
            let Some(image) = session.image.clone() else {
                return;
            };
            session.primary = provider;
            let (original, target) = (session.text.clone(), session.target_language.clone());
            self.present_locked(
                &mut st,
                &PopupViewModel::loading(AIOperation::ScreenTranslate, ""),
            );
            (image, original, target)
        };
        let png = d.gate.run(move || image.to_png()).await;
        if !self.is_current(generation) {
            return;
        }
        let png = match png {
            Ok(png) => png,
            Err(e) => {
                self.log("png encoding failed");
                self.present_error_with_vision(generation, gate_message(&e));
                return;
            }
        };
        let outcome = d
            .runner
            .run_vision(provider, &png, original, target, &snapshot)
            .await;
        self.finish(
            generation,
            &settings,
            &snapshot,
            AIOperation::ScreenTranslate,
            provider,
            outcome,
        )
        .await;
    }
}
