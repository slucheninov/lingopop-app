//! Native menu and cancellable icon animation. No callback performs window or store work.
use crate::{
    commands::ShellState,
    settings_cmds::{self, SettingsPatch},
    state::lock,
    windows,
};
use lingopop_core::{
    host::HostKind,
    hotkey::{Hotkey, Modifier},
    i18n, languages,
    operation::AIOperation,
    settings::Settings,
    ui_prefs::Language,
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
// On Windows the synchronous sink enqueues; its dispatcher posts outside locks in FIFO order.
// Mac keeps the initial template and installs no status-frame dispatcher.
struct NativeIcon {
    #[cfg(not(target_os = "macos"))]
    tx: tokio::sync::mpsc::UnboundedSender<Frame>,
}
impl NativeIcon {
    #[cfg(target_os = "macos")]
    fn new(_app: AppHandle, _tray: TrayIcon) -> Self {
        // Keep the template installed by TrayIconBuilder. Plain set_icon clears its flag.
        Self {}
    }
    #[cfg(not(target_os = "macos"))]
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
        Self { tx }
    }
}
impl IconSink for NativeIcon {
    fn set_icon(&self, frame: Frame) {
        #[cfg(not(target_os = "macos"))]
        let _ = self.tx.send(frame);
        #[cfg(target_os = "macos")]
        let _ = frame;
    }
}
struct StatusState {
    current: TrayStatus,
    generation: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}
struct Icons {
    host: HostKind,
    state: Mutex<StatusState>,
    sink: Arc<dyn IconSink>,
}
impl Icons {
    fn new(sink: Arc<dyn IconSink>) -> Arc<Self> {
        Self::for_host(sink, crate::state::HOST)
    }
    fn for_host(sink: Arc<dyn IconSink>, host: HostKind) -> Arc<Self> {
        Arc::new(Self {
            host,
            state: Mutex::new(StatusState {
                current: TrayStatus::Idle,
                generation: 0,
                task: None,
            }),
            sink,
        })
    }
    fn set(self: &Arc<Self>, status: TrayStatus) {
        // Mac uses a static template: no dispatch, redraw or animation timer.
        if self.host == HostKind::Unix {
            return;
        }
        let mut st = lock(&self.state);
        if status == TrayStatus::Idle && st.current == TrayStatus::Idle && st.task.is_none() {
            return;
        }
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
                        let mut st = lock(&me.state);
                        if st.generation != generation {
                            return;
                        }
                        if status == TrayStatus::Error {
                            st.current = TrayStatus::Idle;
                            st.task = None;
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
/// Characters of a history row before the ellipsis.
pub const MENU_ROW_CHARS: usize = 50;

/// One history row: the copied (original) text only, whitespace and control runs collapsed to
/// one space, cut to `MENU_ROW_CHARS` with `…`. The quick-pick digit of the first ten rows
/// sits at the right edge on both systems: Windows appends `\t&1`..`\t&0` (Win32 draws the
/// text after a tab right-aligned, like the operation chords; the `&` makes the digit the
/// item's mnemonic, so pressing it in the open menu picks the row); macOS shows `⌘1`..`⌘0` as
/// native key equivalents (`history_accelerator`), right-aligned by AppKit.
pub fn menu_row_text(index: usize, original: &str, host: HostKind) -> String {
    let s = original
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = s.chars().take(MENU_ROW_CHARS).collect();
    if s.chars().count() > MENU_ROW_CHARS {
        out.push('…');
    }
    let text = menu_label(&out);
    if host == HostKind::Windows && index < 10 {
        format!("{text}\t&{}", (index + 1) % 10)
    } else {
        text
    }
}

/// macOS key equivalent of history row `index`: `Cmd+1`..`Cmd+9`, `Cmd+0` (Swift parity).
pub fn history_accelerator(index: usize, host: HostKind) -> Option<String> {
    (host == HostKind::Unix && index < 10).then(|| format!("Cmd+{}", (index + 1) % 10))
}

/// macOS key equivalent shown beside an operation (muda accelerator syntax; the key is the
/// W3C `KeyboardEvent.code` that muda parses too). `None` on Windows, where the label carries
/// the chord after a tab instead.
pub fn hotkey_accelerator(hotkey: &Hotkey, host: HostKind) -> Option<String> {
    if host != HostKind::Unix || !hotkey.is_valid() {
        return None;
    }
    let mut parts: Vec<&str> = hotkey
        .modifiers
        .iter()
        .map(|m| match m {
            Modifier::Ctrl => "Control",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
            Modifier::Win => "Cmd",
        })
        .collect();
    parts.push(&hotkey.key);
    Some(parts.join("+"))
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
        #[cfg(target_os = "macos")]
        let idle =
            tauri::image::Image::from_bytes(include_bytes!("../icons/tray/mac-template.png"))?;
        #[cfg(not(target_os = "macos"))]
        let idle = tauri::image::Image::from_bytes(include_bytes!("../icons/tray/idle.png"))?;
        let tray = TrayIconBuilder::new()
            .icon(idle)
            .icon_as_template(cfg!(target_os = "macos"))
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
                                .write_clipboard(entry.original)
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
        let host = crate::state::HOST;
        // Tauri drops an accelerator muda cannot parse (only the shortcut text is lost).
        let add_with =
            |id: &str, label: &str, enabled: bool, accel: Option<&str>| -> tauri::Result<()> {
                menu.append(&MenuItem::with_id(&self.app, id, label, enabled, accel)?)
            };
        let add = |id: &str, label: &str, enabled: bool| add_with(id, label, enabled, None);
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
            add_with(
                &format!("hist:{}", e.id),
                &menu_row_text(i, &e.original, host),
                true,
                history_accelerator(i, host).as_deref(),
            )?;
        }
        menu.append(&PredefinedMenuItem::separator(&self.app)?)?;
        for op in AIOperation::ALL {
            let mut label = t(op.english_name());
            let hotkey = settings.hotkeys.get(&op);
            let accel = hotkey.and_then(|h| hotkey_accelerator(h, host));
            if let (Some(h), HostKind::Windows) = (hotkey, host) {
                label.push('\t');
                label.push_str(&h.display_for(host));
            }
            add_with(
                &format!("op:{}", op.raw_value()),
                &label,
                true,
                accel.as_deref(),
            )?;
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
        let mac = host == HostKind::Unix;
        for (id, key, accel) in [
            ("history", "History…", mac.then_some("Cmd+H")),
            ("settings", "Settings…", mac.then_some("Cmd+Comma")),
            ("about", "About LingoPop", None),
        ] {
            add_with(id, &t(key), true, accel)?;
        }
        menu.append(&PredefinedMenuItem::separator(&self.app)?)?;
        add_with("quit", &t("Quit LingoPop"), true, mac.then_some("Cmd+Q"))?;
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
        (Icons::for_host(sink.clone(), HostKind::Windows), sink)
    }
    async fn advance(ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
        tokio::task::yield_now().await;
    }
    #[tokio::test(start_paused = true)]
    async fn mac_status_changes_never_dispatch_or_start_animation() {
        let sink = Arc::new(Recording::default());
        let icons = Icons::for_host(sink.clone(), HostKind::Unix);
        for status in [TrayStatus::Working, TrayStatus::Error, TrayStatus::Idle] {
            icons.set(status);
            tokio::task::yield_now().await;
            advance(5000).await;
        }
        assert!(sink.0.lock().unwrap().is_empty());
        assert!(icons.state.lock().unwrap().task.is_none());
    }
    #[tokio::test(start_paused = true)]
    async fn repeated_idle_sends_no_frames() {
        let (icons, sink) = rig();
        icons.set(TrayStatus::Idle);
        icons.set(TrayStatus::Idle);
        advance(5000).await;
        assert!(sink.0.lock().unwrap().is_empty());
        icons.set(TrayStatus::Working);
        icons.set(TrayStatus::Idle);
        icons.set(TrayStatus::Idle);
        advance(5000).await;
        assert_eq!(*sink.0.lock().unwrap(), [Frame::Working1, Frame::Idle]);
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
        icons.set(TrayStatus::Idle);
        advance(5000).await;
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
    fn menu_row_text_shows_only_the_original_truncated() {
        let w = HostKind::Windows;
        assert_eq!(
            menu_row_text(0, &"a".repeat(60), w),
            format!("{}…\t&1", "a".repeat(MENU_ROW_CHARS))
        );
        assert_eq!(menu_row_text(10, "a\r\nb", w), "a b");
        assert_eq!(menu_row_text(10, " a\t \r\nb\0\x07c ", w), "a b c");
        assert_eq!(menu_row_text(9, "a", w), "a\t&0");
        assert_eq!(menu_row_text(0, "a", HostKind::Unix), "a");
        assert_eq!(menu_row_text(9, "a", HostKind::Unix), "a");
    }
    #[test]
    fn mac_history_rows_get_cmd_digits() {
        assert_eq!(
            history_accelerator(0, HostKind::Unix).as_deref(),
            Some("Cmd+1")
        );
        assert_eq!(
            history_accelerator(9, HostKind::Unix).as_deref(),
            Some("Cmd+0")
        );
        assert_eq!(history_accelerator(10, HostKind::Unix), None);
        assert_eq!(history_accelerator(0, HostKind::Windows), None);
    }
    #[test]
    fn hotkey_accelerators_are_mac_only_and_parse() {
        let h = Hotkey::new("KeyT", vec![Modifier::Alt]);
        assert_eq!(
            hotkey_accelerator(&h, HostKind::Unix).as_deref(),
            Some("Alt+KeyT")
        );
        assert_eq!(hotkey_accelerator(&h, HostKind::Windows), None);
        let all = Hotkey::new(
            "Digit1",
            vec![
                Modifier::Ctrl,
                Modifier::Alt,
                Modifier::Shift,
                Modifier::Win,
            ],
        );
        let accel = hotkey_accelerator(&all, HostKind::Unix).unwrap();
        for a in [
            accel.as_str(),
            "Alt+KeyT",
            "Cmd+1",
            "Cmd+0",
            "Cmd+H",
            "Cmd+Comma",
            "Cmd+Q",
        ] {
            assert!(a.parse::<muda::accelerator::Accelerator>().is_ok(), "{a}");
        }
    }
    #[test]
    fn menu_label_escapes_ampersand() {
        assert_eq!(menu_label("Rock & Roll"), "Rock && Roll");
        assert!(menu_row_text(0, "A & B", HostKind::Windows).contains("A && B"));
    }
    #[test]
    fn target_items_put_current_first_when_not_quick() {
        let items = target_items("Norwegian");
        assert_eq!(items[0], "Norwegian");
        assert_eq!(target_items("English").len(), languages::quick_list().len());
    }
}
