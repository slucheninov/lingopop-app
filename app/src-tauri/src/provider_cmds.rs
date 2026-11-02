//! Provider settings IPC. The API key travels exactly once, inside `provider_save`'s
//! `ProviderDraft`, straight into the `SecretStore`; no response, event or log line carries it.
//! Every `Err` is a catalog key (already in the launched language), never user text.
use crate::{
    commands::ShellState,
    settings_cmds::settings_view,
    state::{SecretsWriteError, ShellConfig},
};
use lingopop_core::{
    i18n,
    operation::AIOperation,
    provider::{ProviderField, ProviderType},
    provider_draft::{DraftErrors, ProviderDraft, ProviderFields},
};
use lingopop_orchestrator::ConfigSource;
use serde::Serialize;
use tauri::{Emitter, State};

pub const KEY_INVALID: &str = "Invalid provider settings.";
pub const KEY_UNREADABLE: &str = "Provider settings can't be saved because secrets.dat is unreadable. Remove or rename it and enter your keys again.";
pub const KEY_KEY_NOT_SAVED: &str =
    "Couldn't save the API key. Check that the data folder is writable.";

const KEY_GUIDE_BASE: &str =
    "https://github.com/slucheninov/lingopop-app/blob/main/docs/api-keys.md#";

/// Deep link to the provider's section of `docs/api-keys.md` (the headings keep these anchors).
pub fn key_guide_url(provider: ProviderType) -> String {
    let anchor = match provider {
        ProviderType::Claude => "claude",
        ProviderType::OpenAI => "openai",
        ProviderType::Gemini => "gemini",
        ProviderType::Grok => "grok",
        ProviderType::GoogleFree => "google-free",
        ProviderType::GoogleCloud => "google-translate",
        ProviderType::DeepL => "deepl",
        ProviderType::MicrosoftTranslator => "microsoft-translator",
    };
    format!("{KEY_GUIDE_BASE}{anchor}")
}

/// What the form needs about one provider. Holds presence flags only, never key material.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    pub provider: ProviderType,
    pub name: &'static str,
    pub has_key: bool,
    pub requires_key: bool,
    pub base_url: String,
    pub model: String,
    pub referer: String,
    pub region: String,
    pub default_base_url: &'static str,
    pub auto_base_url: bool,
    pub models: &'static [&'static str],
    pub fields: &'static [ProviderField],
    pub operations: Vec<&'static str>,
    pub is_active: bool,
    pub fallback: Option<ProviderType>,
    pub secrets_unavailable: bool,
    pub connections_corrupt: bool,
    pub key_guide_url: String,
}

/// With unavailable or corrupt secrets `connection` yields defaults; the two flags tell the UI so.
pub fn provider_state_of(config: &ShellConfig, provider: ProviderType) -> ProviderState {
    let capabilities = provider.capabilities();
    let connection = config.connection(provider);
    let settings = config.settings();
    ProviderState {
        provider,
        name: provider.display_name(),
        has_key: config.has_api_key(provider),
        requires_key: capabilities.requires_api_key,
        base_url: connection.base_url,
        model: connection.model,
        referer: connection.referer.unwrap_or_default(),
        region: connection.region.unwrap_or_default(),
        default_base_url: capabilities.default_base_url,
        auto_base_url: capabilities.auto_base_url,
        models: capabilities.available_models,
        fields: capabilities.settings_fields,
        operations: AIOperation::ALL
            .into_iter()
            .filter(|op| capabilities.supported_operations.contains(op))
            .map(AIOperation::english_name)
            .collect(),
        is_active: settings.active_provider == provider,
        fallback: settings.fallback(provider),
        secrets_unavailable: !config.secrets_available(),
        connections_corrupt: config.connections_corrupt(),
        key_guide_url: key_guide_url(provider),
    }
}

fn log_write_failure(what: &str, error: &SecretsWriteError) {
    // `SecretsWriteError`'s Display is fixed text; it never carries data.
    log::warn!("provider {what} failed: {error}");
}

/// Validates, then writes the connection (read-modify-write) and then the key, if any.
/// Refuses before touching anything when the secrets file is unavailable or its provider
/// settings are corrupt: `connection` would return defaults and overwrite the user's values.
/// Fields the provider does not show keep their stored value, so an unvalidated one never lands.
pub fn save_draft(
    config: &ShellConfig,
    provider: ProviderType,
    draft: &ProviderDraft,
) -> Result<(), &'static str> {
    if !draft.validate(provider).is_empty() {
        return Err(KEY_INVALID);
    }
    if !config.secrets_available() || config.connections_corrupt() {
        log::warn!("provider save refused: secrets unavailable or unreadable");
        return Err(KEY_UNREADABLE);
    }
    let mut connection = config.connection(provider);
    let shown = provider.capabilities().settings_fields;
    let mut fields = ProviderFields {
        base_url: connection.base_url.clone(),
        model: connection.model.clone(),
        referer: connection.referer.clone().unwrap_or_default(),
        region: connection.region.clone().unwrap_or_default(),
    };
    if shown.contains(&ProviderField::BaseUrl) {
        fields.base_url.clone_from(&draft.fields.base_url);
    }
    if shown.contains(&ProviderField::Model) {
        fields.model.clone_from(&draft.fields.model);
    }
    if shown.contains(&ProviderField::Referer) {
        fields.referer.clone_from(&draft.fields.referer);
    }
    if shown.contains(&ProviderField::Region) {
        fields.region.clone_from(&draft.fields.region);
    }
    fields.apply(provider, &mut connection);
    config
        .set_connection(provider, connection)
        .map_err(|e| match e {
            SecretsWriteError::Unavailable | SecretsWriteError::Corrupt => {
                log_write_failure("save", &e);
                KEY_UNREADABLE
            }
            SecretsWriteError::Store(_) => {
                log_write_failure("save", &e);
                KEY_KEY_NOT_SAVED
            }
        })?;
    if let Some(key) = draft.trimmed_api_key() {
        config.set_api_key(provider, key).map_err(|e| {
            log_write_failure("key save", &e);
            KEY_KEY_NOT_SAVED
        })?;
    }
    Ok(())
}

/// Removes the stored key only; the secrets file keeps every other entry, corrupt ones included.
pub fn clear_key_of(config: &ShellConfig, provider: ProviderType) -> Result<(), &'static str> {
    config.set_api_key(provider, "").map_err(|e| {
        log_write_failure("key clear", &e);
        match e {
            SecretsWriteError::Unavailable | SecretsWriteError::Corrupt => KEY_UNREADABLE,
            SecretsWriteError::Store(_) => KEY_KEY_NOT_SAVED,
        }
    })
}

#[tauri::command]
pub async fn provider_state(
    state: State<'_, ShellState>,
    provider: ProviderType,
) -> Result<ProviderState, String> {
    Ok(provider_state_of(&state.config, provider))
}

/// Key-free: the webview sends only the four visible fields.
#[tauri::command]
pub async fn provider_validate(
    provider: ProviderType,
    fields: ProviderFields,
) -> Result<DraftErrors, String> {
    Ok(fields.validate(provider))
}

#[tauri::command]
pub async fn provider_save(
    app: tauri::AppHandle,
    state: State<'_, ShellState>,
    provider: ProviderType,
    draft: ProviderDraft,
) -> Result<ProviderState, String> {
    let _write = state.settings_write.lock().await;
    let result = save_draft(&state.config, provider, &draft);
    drop(draft);
    // A failed key write still changed the connection: always tell the windows to re-read.
    let _ = app.emit("settings://changed", &settings_view(&state));
    result.map_err(|key| i18n::t(state.ui.language(), key, &[]))?;
    Ok(provider_state_of(&state.config, provider))
}

#[tauri::command]
pub async fn provider_clear_key(
    app: tauri::AppHandle,
    state: State<'_, ShellState>,
    provider: ProviderType,
) -> Result<ProviderState, String> {
    let _write = state.settings_write.lock().await;
    let result = clear_key_of(&state.config, provider);
    let _ = app.emit("settings://changed", &settings_view(&state));
    result.map_err(|key| i18n::t(state.ui.language(), key, &[]))?;
    Ok(provider_state_of(&state.config, provider))
}
