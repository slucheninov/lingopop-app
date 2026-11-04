import { invoke } from '@tauri-apps/api/core';
import { setLanguage } from './i18n';
import type { UiBootstrap } from './settingsTypes';

export const FALLBACK_BOOTSTRAP: UiBootstrap = {
  host: 'windows', language: 'en', launchedLanguage: 'en', catalog: {}, version: '', popupFontSize: 13, initialTab: null,
};

let host: UiBootstrap['host'] = 'windows';
export function currentHost(): UiBootstrap['host'] { return host; }

/**
 * Fetches the language, catalog and font size once per window and installs the catalog.
 * A failure leaves the UI in English with the defaults (nothing is logged: errors may carry user data).
 */
export async function bootstrap(): Promise<UiBootstrap> {
  try {
    const result = await invoke<UiBootstrap>('ui_bootstrap');
    host = result.host === 'unix' ? 'unix' : 'windows';
    setLanguage(result.language, result.catalog);
    document.documentElement.lang = result.language;
    return result;
  } catch {
    return { ...FALLBACK_BOOTSTRAP };
  }
}
