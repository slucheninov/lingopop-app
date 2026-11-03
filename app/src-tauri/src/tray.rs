//! Native menu and cancellable icon animation. No callback performs window or store work.
use crate::{
    commands::ShellState,
    settings_cmds::{self, SettingsPatch},
    windows,
};
use lingopop_core::{
    i18n, languages, operation::AIOperation, settings::Settings, ui_prefs::Language,
};
use lingopop_orchestrator::{ConfigSource, StatusSink, TrayStatus, TriggerSource};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Listener, Manager};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    Idle,
    Working1,
    Working2,
    Error,
}
pub(crate) trait IconSink: Send + Sync {
    fn set_icon(&self, frame: Frame);
}
// The synchronous sink only enqueues. The dispatcher posts to the main thread with no
// orchestrator or icon mutex held; FIFO preserves the order of committed transitions.
struct NativeIcon(tokio::sync::mpsc::UnboundedSender<Frame>);
impl NativeIcon {
    fn new(app: AppHandle, tray: TrayIcon) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tauri::async_runtime::spawn(async move {
            while let Some(frame) = rx.recv().await {
                let bytes: &'static [u8] = match frame {
                    Frame::Idle => include_bytes!("../icons/tray/idle.png"),
                    Frame::Working1 => include_bytes!("../icons/tray/working-1.png"),
                    Frame::Working2 => include_bytes!("../icons/tray/working-2.png"),
                    Frame::Error => include_bytes!("../icons/tray/error.png"),
                };
                let tray = tray.clone();
                if app
                    .run_on_main_thread(move || match tauri::image::Image::from_bytes(bytes) {
                        Ok(icon) => {
                            if tray.set_icon(Some(icon)).is_err() {
                                log::warn!("tray icon update failed");
                            }
                        }
                        Err(_) => log::warn!("tray icon decode failed"),
                    })
                    .is_err()
                {
                    log::warn!("tray icon dispatch failed");
                }
            }
        });
        Self(tx)
    }
}
impl IconSink for NativeIcon {
    fn set_icon(&self, frame: Frame) {
        let _ = self.0.send(frame);
    }
}
struct StatusState {
    current: TrayStatus,
    generation: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}
struct Icons {
    state: Mutex<StatusState>,
    sink: Arc<dyn IconSink>,
}
impl Icons {
    fn new(sink: Arc<dyn IconSink>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(StatusState {
                current: TrayStatus::Idle,
                generation: 0,
                task: None,
            }),
            sink,
        })
    }
    fn set(self: &Arc<Self>, status: TrayStatus) {
        let mut st = self.state.lock().unwrap();
        st.generation += 1;
        let generation = st.generation;
        st.current = status;
        if let Some(task) = st.task.take() {
            task.abort();
        }
        self.sink.set_icon(match status {
            TrayStatus::Idle => Frame::Idle,
            TrayStatus::Working => Frame::Working1,
            TrayStatus::Error => Frame::Error,
        });
        if status == TrayStatus::Idle {
            return;
        }
        let weak = Arc::downgrade(self);
        st.task = Some(
            tokio::runtime::Handle::try_current()
                .unwrap_or_else(|_| tauri::async_runtime::handle().inner().clone())
                .spawn(async move {
                    let mut second = true;
                    loop {
                        tokio::time::sleep(if status == TrayStatus::Error {
                            Duration::from_secs(3)
                        } else {
                            Duration::from_millis(450)
                        })
                        .await;
                        let Some(me) = weak.upgrade() else { return };
                        let mut st = me.state.lock().unwrap();
                        if st.generation != generation {
                            return;
                        }
                        if status == TrayStatus::Error {
                            st.current = TrayStatus::Idle;
                            me.sink.set_icon(Frame::Idle);
                            return;
                        }
                        me.sink.set_icon(if second {
                            Frame::Working2
                        } else {
                            Frame::Working1
                        });
                        second = !second;
                    }
                }),
        );
    }
}
#[derive(Default)]
pub struct TraySink(pub OnceLock<Arc<TrayController>>);
impl StatusSink for TraySink {
    fn status(&self, s: TrayStatus) {
        if let Some(tray) = self.0.get() {
            tray.set_status(s);
        }
    }
}
pub struct TrayController {
    tray: TrayIcon,
    app: AppHandle,
    language: Language,
    icons: Arc<Icons>,
}
impl StatusSink for TrayController {
    fn status(&self, s: TrayStatus) {
        self.set_status(s);
    }
}
pub fn menu_history_count(settings: &Settings) -> usize {
    settings.menu_history_count.min(settings.max_history_count) as usize
}
pub fn menu_label(text: &str) -> String {
    text.replace('&', "&&")
}
pub fn menu_row_text(index: usize, original: &str, translated: &str) -> String {
    fn side(s: &str) -> String {
        let s = s
            .split(|c: char| c.is_whitespace() || c.is_control())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let mut out: String = s.chars().take(40).collect();
        if s.chars().count() > 40 {
            out.push('…');
        }
        out
    }
    let text = menu_label(&format!("{} → {}", side(original), side(translated)));
    if index < 10 {
        format!("&{} {text}", (index + 1) % 10)
    } else {
        text
    }
}
fn target_items(current: &str) -> Vec<String> {
    let mut langs: Vec<String> = languages::quick_list()
        .into_iter()
        .map(String::from)
        .collect();
    if !langs.iter().any(|l| l == current) {
        langs.insert(0, current.into());
    }
    langs
}
impl TrayController {
    pub fn build(app: &AppHandle, language: Language) -> tauri::Result<Arc<Self>> {
        let idle = tauri::image::Image::from_bytes(include_bytes!("../icons/tray/idle.png"))?;
        let tray = TrayIconBuilder::new()
            .icon(idle)
            .tooltip("LingoPop")
            .on_menu_event(|app, event| {
                let app = app.clone();
                let id = event.id.as_ref().to_owned();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<ShellState>();
                    if let Some(raw) = id.strip_prefix("op:") {
                        if let Some(op) = AIOperation::from_raw_value(raw) {
                            state
                                .orchestrator
                                .trigger_from(op, TriggerSource::Tray)
                                .await;
                        }
                    } else if let Some(raw) = id.strip_prefix("hist:") {
                        let entry = uuid::Uuid::parse_str(raw)
                            .ok()
                            .and_then(|id| state.stores.history_entry(id));
                        if let Some(entry) = entry {
                            if state
                                .orchestrator
                                .write_clipboard(entry.translated)
                                .await
                                .is_err()
                            {
                                log::warn!("tray copy failed");
                            }
                        }
                    } else if let Some(lang) = id.strip_prefix("lang:") {
                        let _ = settings_cmds::settings_update(
                            app.clone(),
                            app.state(),
                            SettingsPatch::TargetLanguage(lang.into()),
                        )
                        .await;
                    } else {
                        match id.as_str() {
                            "history" => windows::open_history(&app),
                            "settings" => windows::open_settings(&app, None),
                            "about" => windows::open_settings(&app, Some("about")),
                            "quit" => app.exit(0),
                            _ => {}
                        }
                    }
                });
            })
            .build(app)?;
        let icons = Icons::new(Arc::new(NativeIcon::new(app.clone(), tray.clone())));
        let me = Arc::new(Self {
            tray,
            app: app.clone(),
            language,
            icons,
        });
        me.set_status(TrayStatus::Idle);
        // Setup runs on the main thread: Tauri executes menu operations inline here.
        me.rebuild()?;
        Ok(me)
    }
    pub fn set_status(&self, status: TrayStatus) {
        self.icons.set(status);
    }
    pub fn rebuild(&self) -> tauri::Result<()> {
        let state = self.app.state::<ShellState>();
        let settings = state.config.settings();
        let menu = Menu::new(&self.app)?;
        let add = |id: &str, label: &str, enabled: bool| -> tauri::Result<()> {
            menu.append(&MenuItem::with_id(
                &self.app,
                id,
                label,
                enabled,
                None::<&str>,
            )?)
        };
        let t = |key: &str| i18n::t(self.language, key, &[]);
        add(
            "header",
            &t("Select the clip you want to add to your clipboard"),
            false,
        )?;
        let entries = state.stores.history_entries();
        let count = menu_history_count(&settings);
        if entries.is_empty() || count == 0 {
            add("empty", &t("No recent translations"), false)?;
        }
        for (i, e) in entries.iter().take(count).enumerate() {
            add(
                &format!("hist:{}", e.id),
                &menu_row_text(i, &e.original, &e.translated),
                true,
            )?;
        }
        menu.append(&PredefinedMenuItem::separator(&self.app)?)?;
        for op in AIOperation::ALL {
            let mut label = t(op.english_name());
            if let Some(h) = settings.hotkeys.get(&op) {
                label.push('\t');
                label.push_str(&h.display_string());
            }
            add(&format!("op:{}", op.raw_value()), &label, true)?;
        }
        menu.append(&PredefinedMenuItem::separator(&self.app)?)?;
        let target = Submenu::new(
            &self.app,
            i18n::t(
                self.language,
                "Target: %@",
                &[&menu_label(&settings.target_language)],
            ),
            true,
        )?;
        for lang in target_items(&settings.target_language) {
            target.append(&CheckMenuItem::with_id(
                &self.app,
                format!("lang:{lang}"),
                menu_label(&lang),
                true,
                lang == settings.target_language,
                None::<&str>,
            )?)?;
        }
        menu.append(&target)?;
        for (id, key) in [
            ("history", "History…"),
            ("settings", "Settings…"),
            ("about", "About LingoPop"),
        ] {
            add(id, &t(key), true)?;
        }
        menu.append(&PredefinedMenuItem::separator(&self.app)?)?;
        add("quit", &t("Quit LingoPop"), true)?;
        self.tray.set_menu(Some(menu))
    }
}
pub fn watch(
    app: &AppHandle,
    tray: Arc<TrayController>,
    mut changes: tokio::sync::watch::Receiver<()>,
    tx: tokio::sync::watch::Sender<()>,
) {
    app.listen("settings://changed", move |_| {
        let _ = tx.send(());
    });
    tauri::async_runtime::spawn(async move {
        while changes.changed().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(100)).await;
            changes.borrow_and_update();
            // Native menu calls wait for main-thread roundtrips. Keep those waits
            // off async workers, and never move this entire rebuild to the main loop.
            let tray = tray.clone();
            if !matches!(
                tauri::async_runtime::spawn_blocking(move || tray.rebuild()).await,
                Ok(Ok(()))
            ) {
                log::warn!("tray menu rebuild failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Recording(Mutex<Vec<Frame>>);
    impl IconSink for Recording {
        fn set_icon(&self, frame: Frame) {
            self.0.lock().unwrap().push(frame);
        }
    }
    fn rig() -> (Arc<Icons>, Arc<Recording>) {
        let sink = Arc::new(Recording::default());
        (Icons::new(sink.clone()), sink)
    }
    async fn advance(ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
        tokio::task::yield_now().await;
    }
    #[tokio::test(start_paused = true)]
    async fn working_alternates_frames() {
        let (icons, sink) = rig();
        icons.set(TrayStatus::Working);
        tokio::task::yield_now().await;
        advance(450).await;
        advance(450).await;
        assert_eq!(
            *sink.0.lock().unwrap(),
            [Frame::Working1, Frame::Working2, Frame::Working1]
        );
    }
    #[tokio::test(start_paused = true)]
    async fn working_then_idle_stops_animation() {
        let (icons, sink) = rig();
        icons.set(TrayStatus::Working);
        icons.set(TrayStatus::Idle);
        advance(5000).await;
        assert_eq!(*sink.0.lock().unwrap(), [Frame::Working1, Frame::Idle]);
    }
    #[tokio::test(start_paused = true)]
    async fn error_reverts_to_idle_after_3s() {
        let (icons, sink) = rig();
        icons.set(TrayStatus::Error);
        tokio::task::yield_now().await;
        advance(2999).await;
        assert_eq!(*sink.0.lock().unwrap(), [Frame::Error]);
        advance(1).await;
        assert_eq!(*sink.0.lock().unwrap(), [Frame::Error, Frame::Idle]);
    }
    #[tokio::test(start_paused = true)]
    async fn error_then_working_cancels_revert() {
        let (icons, sink) = rig();
        icons.set(TrayStatus::Error);
        tokio::task::yield_now().await;
        advance(1000).await;
        icons.set(TrayStatus::Working);
        tokio::task::yield_now().await;
        for _ in 0..12 {
            advance(450).await;
        }
        let frames = sink.0.lock().unwrap();
        assert!(!frames.contains(&Frame::Idle));
        assert_eq!(frames.len(), 14);
    }
    #[test]
    fn menu_history_count_is_min() {
        let s = Settings {
            menu_history_count: 20,
            max_history_count: 5,
            ..Settings::default()
        };
        assert_eq!(menu_history_count(&s), 5);
    }
    #[test]
    fn menu_row_text_truncates_and_mnemonics() {
        assert_eq!(
            menu_row_text(0, &"a".repeat(50), " b\nc "),
            format!("&1 {}… → b c", "a".repeat(40))
        );
        assert_eq!(menu_row_text(10, "a\r\nb", "c\td"), "a b → c d");
        assert_eq!(menu_row_text(10, " a\t \r\nb ", "c\0\x07d"), "a b → c d");
        assert!(menu_row_text(9, "a", "b").starts_with("&0 "));
        assert_eq!(menu_row_text(10, "a", "b"), "a → b");
    }
    #[test]
    fn menu_label_escapes_ampersand() {
        assert_eq!(menu_label("Rock & Roll"), "Rock && Roll");
        assert!(menu_row_text(0, "A & B", "C").contains("A && B → C"));
    }
    #[test]
    fn target_items_put_current_first_when_not_quick() {
        let items = target_items("Norwegian");
        assert_eq!(items[0], "Norwegian");
        assert_eq!(target_items("English").len(), languages::quick_list().len());
    }
}
