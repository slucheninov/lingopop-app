import { invoke } from '@tauri-apps/api/core';
import type { DraftErrors, ProviderDraft, ProviderFields, ProviderState, SettingsPatch, SettingsView, UiPatch, UpdateView, UsagePeriod, UsageView } from './settingsTypes';
import type { Provider } from './types';
// A rejection is a catalog key (rendered through t()), never user text. Never log it.
export const getSettings = (): Promise<SettingsView> => invoke<SettingsView>('settings_get');
export const updateSetting = (patch: SettingsPatch): Promise<SettingsView> => invoke<SettingsView>('settings_update', { patch });
export const updateUi = (patch: UiPatch): Promise<SettingsView> => invoke<SettingsView>('ui_update', { patch });
export const relaunch = (): Promise<void> => invoke<void>('app_relaunch');
/** Opens the Login Items pane of System Settings; the shell chooses the target, nothing travels. */
export const openLoginItemsSettings = (): Promise<void> => invoke<void>('login_items_open_settings');
/** Settings regained focus: re-read the login item state (read only; the shell emits `settings://changed` when the approval notice should go). */
export const recheckLoginItem = (): Promise<void> => invoke<void>('login_item_recheck');
export const openUrl = (url: string): Promise<void> => invoke<void>('open_url', { url });
export const getProviderState = (provider: Provider): Promise<ProviderState> => invoke<ProviderState>('provider_state', { provider });
/** Key-free by type: `ProviderFields` has no key. */
export const validateProvider = (provider: Provider, fields: ProviderFields): Promise<DraftErrors> => invoke<DraftErrors>('provider_validate', { provider, fields });
/** The only call that carries an API key. The caller clears its key field before awaiting this. */
export const saveProvider = (provider: Provider, draft: ProviderDraft): Promise<ProviderState> => invoke<ProviderState>('provider_save', { provider, draft });
export const clearProviderKey = (provider: Provider): Promise<ProviderState> => invoke<ProviderState>('provider_clear_key', { provider });
export const getUsage = (period: UsagePeriod): Promise<UsageView> => invoke<UsageView>('usage_get', { period });
export const resetUsage = (): Promise<void> => invoke('usage_reset');
/** Asks GitHub now; the answer is the resulting update state. */
export const checkUpdate = (): Promise<UpdateView> => invoke<UpdateView>('updates_check');
export const installUpdate = (): Promise<UpdateView> => invoke<UpdateView>('updates_install');
export const cancelUpdate = (): Promise<UpdateView> => invoke<UpdateView>('updates_cancel');
/** Read only. */
export const getUpdateState = (): Promise<UpdateView> => invoke<UpdateView>('updates_state');
/** Starts the installed app and exits; a rejection is a catalog key. */
export const restartForUpdate = (): Promise<void> => invoke<void>('updates_restart');
export const UPDATE_EVENT = 'updates://state';
export const RELEASES_PAGE_URL = 'https://github.com/slucheninov/lingopop-app/releases/latest';
export const PRIVACY_URL = 'https://github.com/slucheninov/lingopop-app/blob/main/PRIVACY.md';

export const resetProviderSecrets = (provider: Provider): Promise<ProviderState> => invoke<ProviderState>('provider_reset_secrets', { provider });
