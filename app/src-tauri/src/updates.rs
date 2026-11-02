//! Silent GitHub release check. Everything is injected (transport, clock, deadline), so tests
//! use `FakeTransport` and `ManualClock` and never the network or a runtime timer.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use lingopop_core::http::Transport;
use lingopop_core::time::Clock;
use lingopop_core::updates::{
    fetch_latest_version_with, is_newer, should_check, UPDATE_REQUEST_TIMEOUT,
};
use lingopop_core::VERSION;
use tauri::{AppHandle, Emitter, State};

use crate::state::UiState;

type Deadline = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct UpdateState {
    transport: Arc<dyn Transport>,
    ui: Arc<UiState>,
    clock: Arc<dyn Clock>,
    deadline: Deadline,
}

impl UpdateState {
    pub fn new(transport: Arc<dyn Transport>, ui: Arc<UiState>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            ui,
            clock,
            deadline: Arc::new(|| Box::pin(tokio::time::sleep(UPDATE_REQUEST_TIMEOUT))),
        }
    }

    /// Always asks GitHub (the cache rule is for startup only). Silent on every failure.
    /// Returns the tag only when it is newer than this build; a successful lookup is cached
    /// either way, so the next startup within three hours needs no request.
    pub async fn check_now(&self) -> Option<String> {
        let latest = fetch_latest_version_with(self.transport.as_ref(), (self.deadline)()).await?;
        let checked = self.clock.now().unix_seconds();
        let cached = latest.clone();
        if self
            .ui
            .update(|p| {
                p.latest_version = Some(cached);
                p.last_update_check = Some(checked);
            })
            .is_err()
        {
            log::warn!("update check cache could not be saved");
        }
        is_newer(&latest, VERSION).then_some(latest)
    }

    /// Within the check interval of the last lookup, answers from the cache without a request.
    pub async fn check_at_startup(&self) -> Option<String> {
        let prefs = self.ui.prefs();
        if should_check(prefs.last_update_check, self.clock.now().unix_seconds()) {
            return self.check_now().await;
        }
        prefs.latest_version.filter(|v| is_newer(v, VERSION))
    }
}

/// Startup lookup, then one lookup per interval. The first interval tick fires at once and is
/// skipped. Each newer tag only signals `settings://changed`: the webview re-reads the view.
pub fn spawn_loop(state: Arc<UpdateState>, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let signal = || {
            let _ = app.emit("settings://changed", ());
        };
        if state.check_at_startup().await.is_some() {
            signal();
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs_f64(
            lingopop_core::updates::CHECK_INTERVAL_SECONDS,
        ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            if state.check_now().await.is_some() {
                signal();
            }
        }
    });
}

#[tauri::command]
pub async fn updates_check(
    app: AppHandle,
    state: State<'_, Arc<UpdateState>>,
) -> Result<Option<String>, String> {
    let latest = state.check_now().await;
    if latest.is_some() {
        let _ = app.emit("settings://changed", ());
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_cmds::{settings_view_parts, url_allowed};
    use crate::state::ShellConfig;
    use lingopop_core::errors::TranslationError;
    use lingopop_core::http::{FakeTransport, HttpRequest, HttpResponse};
    use lingopop_core::paths::DataPaths;
    use lingopop_core::time::{ManualClock, Timestamp};
    use lingopop_core::ui_prefs::UiPrefs;
    use lingopop_core::updates::{CHECK_INTERVAL_SECONDS, RELEASES_PAGE_URL};

    const NOW: f64 = 1_800_000_000.0;

    struct Rig {
        dir: tempfile::TempDir,
        ui: Arc<UiState>,
        config: Arc<ShellConfig>,
        clock: Arc<ManualClock>,
        fake: Arc<FakeTransport>,
        state: UpdateState,
    }

    fn rig_with(fake: FakeTransport, deadline: Deadline) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (config, _stores, ui) = crate::task4_tests::rig(dir.path());
        let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(NOW)));
        let fake = Arc::new(fake);
        let mut state = UpdateState::new(fake.clone(), ui.clone(), clock.clone());
        state.deadline = deadline;
        Rig {
            dir,
            ui,
            config,
            clock,
            fake,
            state,
        }
    }

    fn never() -> Deadline {
        Arc::new(|| Box::pin(std::future::pending()))
    }

    fn tag(version: &str) -> FakeTransport {
        FakeTransport::json(200, &format!(r#"{{"tag_name":"{version}"}}"#))
    }

    fn stored(rig: &Rig) -> UiPrefs {
        UiPrefs::load(
            &DataPaths::new(rig.dir.path()).ui_json(),
            &ManualClock::new(Timestamp::from_unix_seconds(NOW)),
        )
        .unwrap()
        .0
    }

    #[tokio::test]
    async fn check_now_stores_cache_and_reports_newer() {
        let rig = rig_with(tag("v9.9.9"), never());
        assert_eq!(rig.state.check_now().await, Some("9.9.9".into()));
        let prefs = stored(&rig);
        assert_eq!(prefs.latest_version.as_deref(), Some("9.9.9"));
        assert_eq!(prefs.last_update_check, Some(NOW));
        assert_eq!(rig.ui.prefs().latest_version.as_deref(), Some("9.9.9"));
    }

    #[tokio::test]
    async fn check_now_silent_on_404_and_network() {
        for fake in [
            FakeTransport::json(404, r#"{"tag_name":"v9.9.9"}"#),
            FakeTransport::new(|_| Err(TranslationError::Network("offline".into()))),
        ] {
            let rig = rig_with(fake, never());
            assert_eq!(rig.state.check_now().await, None);
            assert_eq!(rig.ui.prefs().latest_version, None);
            assert_eq!(rig.ui.prefs().last_update_check, None);
            assert!(!DataPaths::new(rig.dir.path()).ui_json().exists());
        }
    }

    #[tokio::test]
    async fn check_at_startup_uses_cache_within_3h() {
        let rig = rig_with(tag("v8.8.8"), never());
        rig.ui
            .update(|p| {
                p.latest_version = Some("9.9.9".into());
                p.last_update_check = Some(NOW - 100.0);
            })
            .unwrap();
        assert_eq!(rig.state.check_at_startup().await, Some("9.9.9".into()));
        assert!(rig.fake.requests().is_empty());
        // An older cached tag is not an update.
        rig.ui
            .update(|p| p.latest_version = Some("0.0.1".into()))
            .unwrap();
        assert_eq!(rig.state.check_at_startup().await, None);
        assert!(rig.fake.requests().is_empty());
    }

    #[tokio::test]
    async fn check_at_startup_refetches_at_exactly_3h() {
        let rig = rig_with(tag("v8.8.8"), never());
        rig.ui
            .update(|p| {
                p.latest_version = Some("9.9.9".into());
                p.last_update_check = Some(NOW);
            })
            .unwrap();
        rig.clock.advance(CHECK_INTERVAL_SECONDS - 1.0);
        assert_eq!(rig.state.check_at_startup().await, Some("9.9.9".into()));
        assert!(rig.fake.requests().is_empty());
        rig.clock.advance(1.0);
        assert_eq!(rig.state.check_at_startup().await, Some("8.8.8".into()));
        assert_eq!(rig.fake.requests().len(), 1);
        assert_eq!(
            rig.ui.prefs().last_update_check,
            Some(NOW + CHECK_INTERVAL_SECONDS)
        );
    }

    #[tokio::test]
    async fn check_now_ignores_cache() {
        let rig = rig_with(tag("v8.8.8"), never());
        rig.ui
            .update(|p| {
                p.latest_version = Some("9.9.9".into());
                p.last_update_check = Some(NOW - 1.0);
            })
            .unwrap();
        assert_eq!(rig.state.check_now().await, Some("8.8.8".into()));
        assert_eq!(rig.fake.requests().len(), 1);
    }

    struct PendingTransport;
    #[async_trait::async_trait]
    impl Transport for PendingTransport {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, TranslationError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn check_now_times_out_silently() {
        let dir = tempfile::tempdir().unwrap();
        let (_config, _stores, ui) = crate::task4_tests::rig(dir.path());
        let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(NOW)));
        let mut state = UpdateState::new(Arc::new(PendingTransport), ui.clone(), clock);
        state.deadline = Arc::new(|| Box::pin(std::future::ready(())));
        assert_eq!(state.check_now().await, None);
        assert_eq!(ui.prefs().latest_version, None);
        assert_eq!(ui.prefs().last_update_check, None);
    }

    #[test]
    fn settings_view_hides_older_cached_version() {
        let rig = rig_with(tag("v9.9.9"), never());
        for (cached, want) in [("0.0.1", None), ("9.9.9", Some("9.9.9".to_string()))] {
            rig.ui
                .update(|p| p.latest_version = Some(cached.into()))
                .unwrap();
            let view = settings_view_parts(&rig.config, &rig.ui, &[]);
            assert_eq!(view.latest_version, want);
        }
    }

    #[tokio::test]
    async fn equal_or_older_latest_is_none() {
        let version = lingopop_core::VERSION;
        let rig = rig_with(tag(&format!("v{version}")), never());
        assert_eq!(rig.state.check_now().await, None);
        // The cache still records the lookup.
        assert_eq!(rig.ui.prefs().latest_version.as_deref(), Some(version));
        // A prerelease build sees the stable tag of the same triple as an update.
        if version.contains('-') {
            let stable = version.split('-').next().unwrap().to_string();
            let rig = rig_with(tag(&stable), never());
            assert_eq!(rig.state.check_now().await, Some(stable));
        }
    }

    #[test]
    fn open_url_allows_release_and_privacy_pages() {
        assert!(url_allowed(RELEASES_PAGE_URL));
        assert!(url_allowed(
            "https://github.com/slucheninov/lingopop-app/blob/main/PRIVACY.md"
        ));
    }
}
