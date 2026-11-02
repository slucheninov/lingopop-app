import { invoke } from '@tauri-apps/api/core';
import type { DraftErrors, ProviderDraft, ProviderFields, ProviderState, SettingsPatch, SettingsView, UiPatch, UsagePeriod, UsageView } from './settingsTypes';
import type { Provider } from './types';
// A rejection is a catalog key (rendered through t()), never user text. Never log it.
export const getSettings = (): Promise<SettingsView> => invoke<SettingsView>('settings_get');
export const updateSetting = (patch: SettingsPatch): Promise<SettingsView> => invoke<SettingsView>('settings_update', { patch });
export const updateUi = (patch: UiPatch): Promise<SettingsView> => invoke<SettingsView>('ui_update', { patch });
export const relaunch = (): Promise<void> => invoke<void>('app_relaunch');
export const openUrl = (url: string): Promise<void> => invoke<void>('open_url', { url });
export const getProviderState = (provider: Provider): Promise<ProviderState> => invoke<ProviderState>('provider_state', { provider });
/** Key-free by type: `ProviderFields` has no key. */
export const validateProvider = (provider: Provider, fields: ProviderFields): Promise<DraftErrors> => invoke<DraftErrors>('provider_validate', { provider, fields });
/** The only call that carries an API key. The caller clears its key field before awaiting this. */
export const saveProvider = (provider: Provider, draft: ProviderDraft): Promise<ProviderState> => invoke<ProviderState>('provider_save', { provider, draft });
export const clearProviderKey = (provider: Provider): Promise<ProviderState> => invoke<ProviderState>('provider_clear_key', { provider });
export const getUsage = (period: UsagePeriod): Promise<UsageView> => invoke<UsageView>('usage_get', { period });
export const resetUsage = (): Promise<void> => invoke('usage_reset');
