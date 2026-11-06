/// <reference types="vite/client" />
import { expect, test } from 'vitest';

type Catalog = Record<string, string>;
type PlatformCatalog = Record<string, Partial<Record<'pl' | 'uk' | 'de', string>>>;

// Vite globs (no node types needed): every non-test source file as text, the catalogs as JSON.
const sources = import.meta.glob(['../**/*.{svelte,ts}', '!../**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;
const shared = import.meta.glob('../../../i18n/catalogs/*.json', {
  import: 'default',
  eager: true,
}) as Record<string, Catalog>;
const windows = import.meta.glob('../../../i18n/windows.json', {
  import: 'default',
  eager: true,
}) as Record<string, PlatformCatalog>;
const macos = import.meta.glob('../../../i18n/macos.json', {
  import: 'default',
  eager: true,
}) as Record<string, PlatformCatalog>;

const LANGUAGES = ['pl', 'uk', 'de'] as const;
const windowsCatalog = Object.values(windows)[0];
const macosCatalog = Object.values(macos)[0];

function sharedFor(language: (typeof LANGUAGES)[number]): Catalog {
  const entry = Object.entries(shared).find(([path]) => path.endsWith(`/${language}.json`));
  if (!entry) throw new Error(`i18n/catalogs/${language}.json was not found`);
  return entry[1];
}

// t('...') / t("...") with a plain string literal (a template literal or a variable is not a key).
const LITERAL = /\bt\(\s*(?:'((?:\\.|[^'\\])*)'|"((?:\\.|[^"\\])*)")/g;

// Literals that shared frontend code renders on every platform but whose English text is genuinely
// Windows-only (Ctrl / Alt, "Windows Data Protection", "Windows account"). They resolve through
// windows.json only; each needs a platform-aware variant before the Mac UI ships. Keep this list
// narrow: a literal with no Windows-specific wording belongs in the shared catalogs.
const WINDOWS_ONLY_LITERALS = [
  'Click a shortcut field, then press a key combination. Use Ctrl, Alt or Shift plus a key. Esc cancels recording. ✕ removes the shortcut.',
  'Stored locally, encrypted with Windows Data Protection for your user account.',
  'Supports Claude, OpenAI, Gemini, Grok, Google Translate, DeepL, and Microsoft Translator. API keys and provider settings are stored locally, encrypted for your Windows account.',
];

// Literals rendered only by macOS-only UI (the Permissions section, shown when the settings view
// says `platform: "macos"`). They resolve through macos.json only and are not required in
// windows.json. Same guard as above: each must be a real t() literal, in macos.json, not shared.
const MACOS_ONLY_LITERALS = [
  "Press a key combo (⌘ / Option)…",
  "Quit the Swift LingoPop to enable shortcuts in LingoPop Preview.",
  "Suspended while Swift LingoPop is running.",
  "Click a shortcut field, then press a key combination. Use ⌘, Option, Control or Shift plus a key. Esc cancels recording. ✕ removes the shortcut.",

  'Accessibility',
  'Screen Recording',
  'Permissions',
  'Required to read selected text and paste results.',
  'Required for Screen Translate.',
  'Granted',
  'Not requested yet',
  'Denied',
  'Restricted by a profile or administrator',
  'Request',
  'Open Settings',
  'Re-check',
  'After granting access, fully quit and relaunch LingoPop.',
  'Allow LingoPop in System Settings > Login Items to start it at login.',
  'Open Login Items settings',
  "Launch at login isn't available until LingoPop runs from the Applications folder. Move it there and open it again.",
  "Launch at login isn't available for this copy of LingoPop. Reinstall it in the Applications folder and open it again.",
  "Couldn't change Launch at login. Try again.",
];

function literalsInSrc(): Set<string> {
  const keys = new Set<string>();
  for (const text of Object.values(sources)) {
    for (const match of text.matchAll(LITERAL)) {
      keys.add((match[1] ?? match[2]).replace(/\\(['"\\])/g, '$1'));
    }
  }
  return keys;
}

test('every_t_literal_in_src_has_a_translation_on_both_platforms', () => {
  const keys = literalsInSrc();
  const missing: string[] = [];
  expect(keys.size).toBeGreaterThan(10);
  for (const key of keys) {
    const windowsOnly = WINDOWS_ONLY_LITERALS.includes(key);
    const macosOnly = MACOS_ONLY_LITERALS.includes(key);
    for (const language of LANGUAGES) {
      const inShared = key in sharedFor(language);
      if (!inShared && !macosOnly && !windowsCatalog[key]?.[language]) missing.push(`windows ${language}: ${key}`);
      if (!inShared && !windowsOnly && !macosCatalog[key]?.[language]) {
        missing.push(`macos ${language}: ${key}`);
      }
    }
  }
  expect(missing).toEqual([]);
});

test('windows_only_literal_exemptions_are_real_and_not_shared', () => {
  const keys = literalsInSrc();
  for (const key of WINDOWS_ONLY_LITERALS) {
    expect(keys.has(key), `not a t() literal in src: ${key}`).toBe(true);
    expect(windowsCatalog[key], `not in windows.json: ${key}`).toBeDefined();
    for (const language of LANGUAGES) expect(key in sharedFor(language)).toBe(false);
  }
});

test('macos_only_literal_exemptions_are_real_and_not_shared', () => {
  const keys = literalsInSrc();
  for (const key of MACOS_ONLY_LITERALS) {
    expect(keys.has(key), `not a t() literal in src: ${key}`).toBe(true);
    expect(macosCatalog[key], `not in macos.json: ${key}`).toBeDefined();
    for (const language of LANGUAGES) expect(key in sharedFor(language)).toBe(false);
  }
});

test('platform_catalogs_have_all_three_languages_for_every_key', () => {
  for (const catalog of [windowsCatalog, macosCatalog]) {
    for (const [key, values] of Object.entries(catalog)) {
      expect(Object.keys(values).sort(), key).toEqual(['de', 'pl', 'uk']);
    }
  }
});

test('shared_languages_have_the_same_keys_and_no_platform_key', () => {
  const plKeys = Object.keys(sharedFor('pl')).sort();
  for (const language of LANGUAGES) {
    expect(Object.keys(sharedFor(language)).sort(), language).toEqual(plKeys);
  }
  for (const key of [...Object.keys(windowsCatalog), ...Object.keys(macosCatalog)]) {
    expect(plKeys, key).not.toContain(key);
  }
});
