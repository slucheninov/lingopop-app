import { invoke } from '@tauri-apps/api/core';
import type { SettingsPatch, SettingsView, UiPatch } from './settingsTypes';
// A rejection is a catalog key (rendered through t()), never user text. Never log it.
export const getSettings = (): Promise<SettingsView> => invoke<SettingsView>('settings_get');
export const updateSetting = (patch: SettingsPatch): Promise<SettingsView> => invoke<SettingsView>('settings_update', { patch });
export const updateUi = (patch: UiPatch): Promise<SettingsView> => invoke<SettingsView>('ui_update', { patch });
export const relaunch = (): Promise<void> => invoke<void>('app_relaunch');
export const openUrl = (url: string): Promise<void> => invoke<void>('open_url', { url });
