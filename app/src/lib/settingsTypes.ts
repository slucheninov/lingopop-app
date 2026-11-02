// Mirrors app/src-tauri/src/settings_cmds.rs (serde wire shapes). API keys never appear here: only presence flags.
import type { Operation, Provider } from './types';

export type Language = 'en' | 'pl' | 'uk' | 'de';
export type AppLanguage = 'system' | Language;
export type Modifier = 'ctrl' | 'alt' | 'shift' | 'win';
export type HotkeyStatus = 'registered' | 'conflict' | 'invalid' | 'none';
export type TabId = 'main' | 'providers' | 'shortcuts' | 'usage' | 'magic' | 'about';

/** Core `Hotkey` as it travels in `SettingsPatch::Hotkey`: W3C `KeyboardEvent.code` + modifiers. */
export interface Hotkey { key: string; modifiers: Modifier[] }
export interface HotkeyView { key: string; modifiers: Modifier[]; display: string }
export interface ProviderSummary { provider: Provider; name: string; hasKey: boolean; requiresKey: boolean; fallback: Provider | null }

export interface SettingsView {
  activeProvider: Provider;
  fallbacks: Partial<Record<Provider, Provider>>;
  targetLanguage: string;
  hotkeys: Partial<Record<Operation, HotkeyView>>;
  hotkeyStatus: Partial<Record<Operation, HotkeyStatus>>;
  launchAtLogin: boolean;
  maxInputChars: number;
  maxHistoryCount: number;
  menuHistoryCount: number;
  useAutoDetect: boolean;
  languagePairs: Record<string, string>;
  autoCopyOCR: boolean;
  magicPrompt: string;
  quickLanguages: string[];
  popupFontSize: number;
  appLanguage: AppLanguage;
  launchedLanguage: Language;
  needsRelaunch: boolean;
  latestVersion: string | null;
  providers: ProviderSummary[];
}

export type SettingsPatch =
  | { field: 'targetLanguage'; value: string }
  | { field: 'useAutoDetect'; value: boolean }
  | { field: 'languagePairs'; value: Record<string, string> }
  | { field: 'maxInputChars' | 'maxHistoryCount' | 'menuHistoryCount'; value: number }
  | { field: 'launchAtLogin' | 'autoCopyOCR'; value: boolean }
  | { field: 'magicPrompt'; value: string }
  | { field: 'activeProvider'; value: Provider }
  | { field: 'fallback'; value: { provider: Provider; fallback: Provider | null } }
  | { field: 'hotkey'; value: { operation: Operation; hotkey: Hotkey | null } };

export type UiPatch = { field: 'popupFontSize'; value: number } | { field: 'appLanguage'; value: AppLanguage };

export interface UiBootstrap {
  language: Language;
  launchedLanguage: Language;
  catalog: Record<string, string>;
  version: string;
  popupFontSize: number;
  initialTab: TabId | null;
}

/** Event payload of `ui://changed`. */
export interface UiChanged { popupFontSize: number; appLanguage: AppLanguage }

export type ProviderFieldName = 'apiKey' | 'baseUrl' | 'model' | 'referer' | 'region';
/** The four key-free visible fields: the only thing `provider_validate` receives. */
export interface ProviderFields { baseUrl: string; model: string; referer: string; region: string }
/** `provider_save` argument: the one place an API key leaves the webview. Never store or log it. */
export interface ProviderDraft extends ProviderFields { apiKey: string }
/** Catalog keys (English text) per invalid field; `null` = valid. */
export interface DraftErrors { baseUrl: string | null; referer: string | null; region: string | null }
/** Mirrors `provider_cmds.rs` `ProviderState`: presence flags only, never key material. */
export interface ProviderState {
  provider: Provider;
  name: string;
  hasKey: boolean;
  requiresKey: boolean;
  baseUrl: string;
  model: string;
  referer: string;
  region: string;
  defaultBaseUrl: string;
  autoBaseUrl: boolean;
  models: string[];
  fields: ProviderFieldName[];
  operations: string[];
  isActive: boolean;
  fallback: Provider | null;
  secretsUnavailable: boolean;
  connectionsCorrupt: boolean;
  keyGuideUrl: string;
}
