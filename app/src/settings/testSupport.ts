import { vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { SettingsView, UiBootstrap } from '../lib/settingsTypes';

export const baseView: SettingsView = {
  activeProvider: 'Google (Free)', fallbacks: {}, targetLanguage: 'English', hotkeys: {}, swiftRunning: false, hotkeyStatus: {},
  launchAtLogin: false, maxInputChars: 0, maxHistoryCount: 20, menuHistoryCount: 5, maxClipboardHistoryCount: 100, menuClipboardHistoryCount: 10, useAutoDetect: false,
  languagePairs: {}, autoCopyOCR: false, magicPrompt: '', defaultMagicPrompt: 'Default prompt {{targetLanguage}}', quickLanguages: ['English', 'Russian', 'Ukrainian', 'Polish'],
  popupFontSize: 13, appLanguage: 'system', launchedLanguage: 'en', needsRelaunch: false, version: '0.2.0', latestVersion: null, providers: [], platform: 'windows',
};
export const baseBootstrap: UiBootstrap = {
  host: 'windows',
  language: 'en', launchedLanguage: 'en', catalog: {}, version: '0.0.0', popupFontSize: 13, initialTab: null,
};

export const listeners = new Map<string, (event: { payload: unknown }) => void>();

/** Routes the mocked `invoke` by command; a value that is an Error is thrown, a function is called with the args. */
export function mockIpc(overrides: Record<string, unknown> = {}): void {
  const table: Record<string, unknown> = { ui_bootstrap: baseBootstrap, settings_get: baseView, settings_update: baseView, ui_update: baseView, ...overrides };
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    const value = table[command];
    if (typeof value === 'function') return (value as (a: unknown) => unknown)(args);
    if (typeof value === 'string' && (command === 'app_relaunch' || command.startsWith('permissions_') || command.startsWith('login_items_'))) throw value;
    if (value instanceof Error) throw value;
    return value;
  });
  listeners.clear();
  vi.mocked(listen).mockImplementation((async (name: string, callback: (event: { payload: unknown }) => void) => {
    listeners.set(name, callback);
    return vi.fn();
  }) as unknown as typeof listen);
}
