//! Settings IPC contains presence flags, never API key material.
use crate::{
    commands::ShellState,
    state::{ShellConfig, ShellStores, UiState},
};
use lingopop_core::{
    hotkey::{Hotkey, Modifier},
    i18n,
    operation::AIOperation,
    provider::ProviderType,
    settings::{sanitize_magic_prompt, Settings, SettingsError},
    ui_prefs::{AppLanguage, Language, POPUP_FONT_RANGE},
};
use lingopop_orchestrator::ConfigSource;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tauri::{Emitter, State};

pub trait SettingsHooks: Send + Sync {
    fn launch_at_login(&self, enabled: bool);
    fn hotkeys(&self, hotkeys: &BTreeMap<AIOperation, Hotkey>);
    /// What the last launch-at-login update left for the user to act on, if anything.
    fn login_issue(&self) -> Option<LoginIssue> {
        None
    }
    /// Re-reads the login item's state (read only, never registers) and drops a pending
    /// "needs approval" issue once the system reports it enabled. Blocking: call it off the
    /// main thread. Signals through the same callback as `launch_at_login` when it changed.
    fn recheck_login_issue(&self) {}
}
pub struct NoSettingsHooks;
impl SettingsHooks for NoSettingsHooks {
    fn launch_at_login(&self, _: bool) {}
    fn hotkeys(&self, _: &BTreeMap<AIOperation, Hotkey>) {}
}
/// The outcome of the last launch-at-login update that the Main tab shows under the switch. Fixed
/// ids only; the webview maps them to catalog sentences. macOS: the login item needs approval in
/// System Settings, cannot work from here (not an installed bundle), or failed otherwise. A
/// Windows Run-key failure stays a log line and is never one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginIssue {
    RequiresApproval,
    Unavailable,
    /// The app is in an installed bundle but the system cannot find its login item service.
    ServiceNotFound,
    Failed,
}
impl LoginIssue {
    /// `None` for an error the UI does not show (every Windows failure).
    pub fn classify(error: &lingopop_platform::PlatformError) -> Option<Self> {
        use lingopop_platform::login_item::{
            UNSUPPORTED_LOGIN_ITEM, UNSUPPORTED_LOGIN_ITEM_NOT_FOUND,
        };
        use lingopop_platform::PlatformError;
        match error {
            PlatformError::LoginItemRequiresApproval => Some(Self::RequiresApproval),
            PlatformError::Unsupported(what) if *what == UNSUPPORTED_LOGIN_ITEM => {
                Some(Self::Unavailable)
            }
            PlatformError::Unsupported(what) if *what == UNSUPPORTED_LOGIN_ITEM_NOT_FOUND => {
                Some(Self::ServiceNotFound)
            }
            _ if cfg!(target_os = "macos") => Some(Self::Failed),
            _ => None,
        }
    }
}
/// Opens the Login Items pane of System Settings. The target is chosen by the platform; the
/// webview supplies nothing. Rejects with a catalog key in the launched language.
pub fn open_login_items_settings(
    login: &dyn lingopop_platform::LoginItem,
    language: Language,
) -> Result<(), String> {
    login.open_settings().map_err(|_| {
        log::warn!("login items settings could not be opened");
        i18n::t_for(
            CATALOG_PLATFORM,
            language,
            "Couldn't open System Settings.",
            &[],
        )
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyStatus {
    Registered,
    SwiftSuspended,
    Conflict,
    Invalid,
    None,
}
#[derive(Serialize)]
pub struct HotkeyView {
    pub key: String,
    pub modifiers: Vec<Modifier>,
    pub display: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub provider: ProviderType,
    pub name: &'static str,
    pub has_key: bool,
    pub requires_key: bool,
    pub fallback: Option<ProviderType>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub active_provider: ProviderType,
    pub fallbacks: BTreeMap<ProviderType, ProviderType>,
    pub target_language: String,
    pub hotkeys: BTreeMap<AIOperation, HotkeyView>,
    pub swift_running: bool,
    pub hotkey_status: BTreeMap<AIOperation, HotkeyStatus>,
    pub launch_at_login: bool,
    pub max_input_chars: u32,
    pub max_history_count: u32,
    pub menu_history_count: u32,
    pub max_clipboard_history_count: u32,
    pub menu_clipboard_history_count: u32,
    pub use_auto_detect: bool,
    pub language_pairs: BTreeMap<String, String>,
    #[serde(rename = "autoCopyOCR")]
    pub auto_copy_ocr: bool,
    pub magic_prompt: String,
    pub default_magic_prompt: &'static str,
    pub quick_languages: Vec<&'static str>,
    pub popup_font_size: u32,
    pub app_language: AppLanguage,
    pub launched_language: Language,
    pub needs_relaunch: bool,
    pub version: &'static str,
    pub latest_version: Option<String>,
    pub providers: Vec<ProviderSummary>,
    /// `macos` or `windows`: the frontend shows the Permissions section on macOS only.
    pub platform: &'static str,
    /// Set while the last launch-at-login update needs the user (see [`LoginIssue`]); omitted
    /// from the JSON otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_item_issue: Option<LoginIssue>,
}
pub fn settings_view(state: &ShellState) -> SettingsView {
    let mut view = settings_view_parts(
        &state.config,
        &state.ui,
        &state.platform.system.ui_languages(),
    );
    view.hotkey_status = state.hotkeys.status();
    view.swift_running = state.hotkeys.swift_blocked();
    view.login_item_issue = state.hooks.login_issue();
    view
}
pub fn settings_view_parts(
    config: &ShellConfig,
    ui: &UiState,
    languages: &[String],
) -> SettingsView {
    let s = config.settings();
    let p = ui.prefs();
    let providers = ProviderType::ALL
        .into_iter()
        .map(|provider| ProviderSummary {
            provider,
            name: provider.display_name(),
            has_key: config.has_api_key(provider),
            requires_key: provider.capabilities().requires_api_key,
            fallback: s.fallback(provider),
        })
        .collect();
    let hotkeys = s
        .hotkeys
        .iter()
        .map(|(op, h)| {
            (
                *op,
                HotkeyView {
                    key: h.key.clone(),
                    modifiers: h.modifiers.clone(),
                    display: h.display_for(crate::state::HOST),
                },
            )
        })
        .collect();
    SettingsView {
        active_provider: s.active_provider,
        fallbacks: s.fallbacks.0,
        target_language: s.target_language,
        hotkeys,
        swift_running: false,
        hotkey_status: AIOperation::ALL
            .into_iter()
            .map(|op| (op, HotkeyStatus::None))
            .collect(),
        launch_at_login: s.launch_at_login,
        max_input_chars: s.max_input_chars,
        max_history_count: s.max_history_count,
        menu_history_count: s.menu_history_count,
        max_clipboard_history_count: s.max_clipboard_history_count,
        menu_clipboard_history_count: s.menu_clipboard_history_count,
        use_auto_detect: s.use_auto_detect,
        language_pairs: s.language_pairs,
        auto_copy_ocr: s.auto_copy_ocr,
        magic_prompt: s.magic_prompt,
        default_magic_prompt: lingopop_core::prompts::DEFAULT_MAGIC_PROMPT,
        quick_languages: lingopop_core::languages::quick_list(),
        popup_font_size: p.popup_font_size,
        app_language: p.app_language,
        launched_language: ui.launched,
        needs_relaunch: p.app_language.needs_relaunch(ui.launched, languages),
        version: lingopop_core::VERSION,
        // An older or equal cached tag is not an update.
        latest_version: p
            .latest_version
            .filter(|v| lingopop_core::updates::is_newer(v, lingopop_core::VERSION)),
        providers,
        platform: if cfg!(target_os = "macos") {
            "macos"
        } else {
            "windows"
        },
        login_item_issue: None,
    }
}
#[derive(Deserialize)]
#[serde(tag = "field", content = "value", rename_all = "camelCase")]
pub enum SettingsPatch {
    TargetLanguage(String),
    UseAutoDetect(bool),
    LanguagePairs(BTreeMap<String, String>),
    MaxInputChars(u32),
    MaxHistoryCount(u32),
    MenuHistoryCount(u32),
    MaxClipboardHistoryCount(u32),
    MenuClipboardHistoryCount(u32),
    LaunchAtLogin(bool),
    #[serde(rename = "autoCopyOCR")]
    AutoCopyOcr(bool),
    MagicPrompt(String),
    ActiveProvider(ProviderType),
    Fallback {
        provider: ProviderType,
        fallback: Option<ProviderType>,
    },
    Hotkey {
        operation: AIOperation,
        hotkey: Option<Hotkey>,
    },
}
impl std::fmt::Debug for SettingsPatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MagicPrompt(s) => write!(f, "MagicPrompt({} chars)", s.chars().count()),
            Self::LanguagePairs(m) => write!(f, "LanguagePairs({} pairs)", m.len()),
            Self::TargetLanguage(_) => f.write_str("TargetLanguage(<redacted>)"),
            _ => f.write_str(match self {
                Self::UseAutoDetect(_) => "UseAutoDetect",
                Self::MaxInputChars(_) => "MaxInputChars",
                Self::MaxHistoryCount(_) => "MaxHistoryCount",
                Self::MenuHistoryCount(_) => "MenuHistoryCount",
                Self::MaxClipboardHistoryCount(_) => "MaxClipboardHistoryCount",
                Self::MenuClipboardHistoryCount(_) => "MenuClipboardHistoryCount",
                Self::LaunchAtLogin(_) => "LaunchAtLogin",
                Self::AutoCopyOcr(_) => "AutoCopyOcr",
                Self::ActiveProvider(_) => "ActiveProvider",
                Self::Fallback { .. } => "Fallback",
                Self::Hotkey { .. } => "Hotkey",
                _ => unreachable!(),
            }),
        }
    }
}
#[derive(Deserialize, Debug)]
#[serde(tag = "field", content = "value", rename_all = "camelCase")]
pub enum UiPatch {
    PopupFontSize(u32),
    AppLanguage(AppLanguage),
}
pub fn error_with_args(key: &str, args: &[&str]) -> String {
    std::iter::once(key)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join("|")
}
pub fn validate_patch(
    s: &Settings,
    patch: &SettingsPatch,
    language: Language,
) -> Result<(), String> {
    if let SettingsPatch::Hotkey {
        operation,
        hotkey: Some(h),
    } = patch
    {
        if !h.is_valid() {
            return Err(error_with_args("This shortcut can't be registered.", &[]));
        }
        if let Some(other) = lingopop_core::hotkey::duplicate_of(&s.hotkeys, h, *operation) {
            // The operation name is a mac catalog key: show it in the launched language.
            let name = i18n::t(language, other.english_name(), &[]);
            return Err(error_with_args("Already used by %@.", &[name.as_str()]));
        }
    }
    Ok(())
}
pub fn apply_patch(s: &mut Settings, patch: SettingsPatch) {
    match patch {
        SettingsPatch::TargetLanguage(v) => s.target_language = v,
        SettingsPatch::UseAutoDetect(v) => s.use_auto_detect = v,
        SettingsPatch::LanguagePairs(v) => s.language_pairs = v,
        SettingsPatch::MaxInputChars(v) => s.max_input_chars = v,
        SettingsPatch::MaxHistoryCount(v) => s.max_history_count = v,
        SettingsPatch::MenuHistoryCount(v) => s.menu_history_count = v,
        SettingsPatch::MaxClipboardHistoryCount(v) => s.max_clipboard_history_count = v.min(1000),
        SettingsPatch::MenuClipboardHistoryCount(v) => s.menu_clipboard_history_count = v.min(50),
        SettingsPatch::LaunchAtLogin(v) => s.launch_at_login = v,
        SettingsPatch::AutoCopyOcr(v) => s.auto_copy_ocr = v,
        SettingsPatch::MagicPrompt(v) => s.magic_prompt = sanitize_magic_prompt(&v),
        SettingsPatch::ActiveProvider(v) => s.active_provider = v,
        SettingsPatch::Fallback { provider, fallback } => s.set_fallback(fallback, provider),
        SettingsPatch::Hotkey { operation, hotkey } => s.set_hotkey(operation, hotkey),
    }
}
/// Caller holds settings_write through validation, save and committed-state effects.
pub(crate) fn save_patch(
    config: &ShellConfig,
    patch: SettingsPatch,
    language: Language,
) -> Result<(), String> {
    validate_patch(&config.settings(), &patch, language)?;
    config
        .update_settings(|s| apply_patch(s, patch))
        .map_err(|e| {
            match e {
                SettingsError::Io(e) => log::warn!("settings save failed: {:?}", e.kind()),
                _ => log::warn!("settings save failed: encoding"),
            };
            i18n::t(language, "Couldn't save settings.", &[])
        })?;
    Ok(())
}
/// Called under ShellState.settings_write after the save, with no std config guard held.
/// Re-read the committed settings so no effect consumes an earlier save result.
pub(crate) fn apply_settings_effects(
    config: &ShellConfig,
    stores: &ShellStores,
    hooks: &dyn SettingsHooks,
    trim: bool,
    launch: bool,
    hotkeys: bool,
) {
    let committed = config.settings();
    if trim {
        stores.trim_history(committed.max_history_count);
    }
    if launch {
        hooks.launch_at_login(committed.launch_at_login);
    }
    if hotkeys {
        hooks.hotkeys(&committed.hotkeys);
    }
}
/// The "Open Login Items settings" button. Runs on the blocking pool, never on the main thread.
#[tauri::command]
pub async fn login_items_open_settings(state: State<'_, ShellState>) -> Result<(), String> {
    let (login, language) = (state.platform.login_item.clone(), state.ui.language());
    tauri::async_runtime::spawn_blocking(move || {
        open_login_items_settings(login.as_ref(), language)
    })
    .await
    .map_err(|_| {
        i18n::t_for(
            CATALOG_PLATFORM,
            language,
            "Couldn't open System Settings.",
            &[],
        )
    })?
}
/// Settings regained focus: the user may have approved the login item in System Settings. Runs on
/// the blocking pool; takes no argument and changes nothing but the remembered issue.
#[tauri::command]
pub async fn login_item_recheck(state: State<'_, ShellState>) -> Result<(), String> {
    let hooks = state.hooks.clone();
    // A join failure only means no re-check happened; the next focus tries again.
    let _ = tauri::async_runtime::spawn_blocking(move || hooks.recheck_login_issue()).await;
    Ok(())
}
#[tauri::command]
pub async fn settings_get(state: State<'_, ShellState>) -> Result<SettingsView, String> {
    Ok(settings_view(&state))
}
#[tauri::command]
pub async fn settings_update(
    app: tauri::AppHandle,
    state: State<'_, ShellState>,
    patch: SettingsPatch,
) -> Result<SettingsView, String> {
    let _write = state.settings_write.lock().await;
    let trim = matches!(patch, SettingsPatch::MaxHistoryCount(_));
    let clipboard_trim = matches!(patch, SettingsPatch::MaxClipboardHistoryCount(_));
    let launch = matches!(patch, SettingsPatch::LaunchAtLogin(_));
    let hotkeys = matches!(patch, SettingsPatch::Hotkey { .. });
    save_patch(&state.config, patch, state.ui.language())?;
    apply_settings_effects(
        &state.config,
        &state.stores,
        state.hooks.as_ref(),
        trim,
        launch,
        hotkeys,
    );
    let clipboard_result = if clipboard_trim {
        state
            .stores
            .clipboard
            .trim(state.config.settings().max_clipboard_history_count)
    } else {
        Ok(())
    };
    let view = settings_view(&state);
    let _ = app.emit("settings://changed", &view);
    clipboard_result.map_err(|_| {
        i18n::t(
            state.ui.language(),
            "Clipboard history couldn't be updated.",
            &[],
        )
    })?;
    Ok(view)
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiChanged {
    popup_font_size: u32,
    app_language: AppLanguage,
}
#[tauri::command]
pub async fn ui_update(
    app: tauri::AppHandle,
    state: State<'_, ShellState>,
    patch: UiPatch,
) -> Result<SettingsView, String> {
    let _write = state.settings_write.lock().await;
    state
        .ui
        .update(|p| match patch {
            UiPatch::PopupFontSize(v) => {
                p.popup_font_size = v.clamp(*POPUP_FONT_RANGE.start(), *POPUP_FONT_RANGE.end())
            }
            UiPatch::AppLanguage(v) => p.app_language = v,
        })
        .map_err(|_| {
            log::warn!("UI preferences save failed");
            i18n::t(state.ui.language(), "Couldn't save settings.", &[])
        })?;
    let view = settings_view(&state);
    let _ = app.emit(
        "ui://changed",
        UiChanged {
            popup_font_size: view.popup_font_size,
            app_language: view.app_language,
        },
    );
    Ok(view)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiBootstrap {
    pub host: &'static str,
    pub language: &'static str,
    pub launched_language: Language,
    pub catalog: &'static HashMap<String, String>,
    pub version: &'static str,
    pub popup_font_size: u32,
    pub initial_tab: Option<String>,
}
/// The platform file of the catalogs this build ships to the webview.
#[cfg(target_os = "macos")]
pub(crate) const CATALOG_PLATFORM: i18n::CatalogPlatform = i18n::CatalogPlatform::Macos;
#[cfg(not(target_os = "macos"))]
pub(crate) const CATALOG_PLATFORM: i18n::CatalogPlatform = i18n::CatalogPlatform::Windows;

#[tauri::command]
pub async fn ui_bootstrap(
    window: tauri::Window,
    state: State<'_, ShellState>,
) -> Result<UiBootstrap, String> {
    let lang = state.ui.language();
    Ok(UiBootstrap {
        host: match crate::state::HOST {
            lingopop_core::host::HostKind::Unix => "unix",
            lingopop_core::host::HostKind::Windows => "windows",
        },
        language: lang.code(),
        launched_language: lang,
        catalog: i18n::catalog_for(CATALOG_PLATFORM, lang),
        version: lingopop_core::VERSION,
        popup_font_size: state.ui.prefs().popup_font_size,
        initial_tab: if window.label() == crate::windows::SETTINGS_LABEL {
            state.ui.take_pending_tab()
        } else {
            None
        },
    })
}
pub fn url_allowed(url: &str) -> bool {
    if url == lingopop_core::updates::RELEASES_PAGE_URL {
        return true;
    }
    // Reject encoded traversal and separators as well, since browsers normalize them.
    let lower = url.to_ascii_lowercase();
    url.starts_with("https://github.com/slucheninov/lingopop-app/blob/main/")
        && !url.contains("..")
        && !url.contains('\\')
        && !lower.contains("%2e")
        && !lower.contains("%2f")
        && !lower.contains("%5c")
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}
#[tauri::command]
pub async fn open_url(state: State<'_, ShellState>, url: String) -> Result<(), String> {
    if !url_allowed(&url) {
        log::warn!("open_url refused");
        return Err(i18n::t(state.ui.language(), "Link not allowed.", &[]));
    }
    state
        .platform
        .system
        .open_url(&url)
        .map_err(|e| e.to_string())
}
