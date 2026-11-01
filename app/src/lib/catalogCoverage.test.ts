/// <reference types="vite/client" />
import { expect, test } from 'vitest';

type Catalog = Record<string, string>;
type WindowsCatalog = Record<string, Partial<Record<'pl' | 'uk' | 'de', string>>>;

// Vite globs (no node types needed): every non-test source file as text, the shared catalogs as JSON.
const sources = import.meta.glob(['../**/*.{svelte,ts}', '!../**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;
const generated = import.meta.glob('../../../i18n/generated/*.json', {
  import: 'default',
  eager: true,
}) as Record<string, Catalog>;
const windows = import.meta.glob('../../../i18n/windows.json', {
  import: 'default',
  eager: true,
}) as Record<string, WindowsCatalog>;

const LANGUAGES = ['pl', 'uk', 'de'] as const;
const windowsCatalog = Object.values(windows)[0];

function generatedFor(language: (typeof LANGUAGES)[number]): Catalog {
  const entry = Object.entries(generated).find(([path]) => path.endsWith(`/${language}.json`));
  if (!entry) throw new Error(`i18n/generated/${language}.json was not found`);
  return entry[1];
}

// t('...') / t("...") with a plain string literal (a template literal or a variable is not a key).
const LITERAL = /\bt\(\s*(?:'((?:\\.|[^'\\])*)'|"((?:\\.|[^"\\])*)")/g;

test('every_t_literal_in_src_has_a_translation', () => {
  const missing: string[] = [];
  let found = 0;
  for (const text of Object.values(sources)) {
    for (const match of text.matchAll(LITERAL)) {
      found++;
      const key = (match[1] ?? match[2]).replace(/\\(['"\\])/g, '$1');
      for (const language of LANGUAGES) {
        if (!(key in generatedFor(language)) && !windowsCatalog[key]?.[language]) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
  }
  expect(found).toBeGreaterThan(10);
  expect(missing).toEqual([]);
});

test('windows_catalog_has_all_three_languages_for_every_key', () => {
  for (const [key, values] of Object.entries(windowsCatalog)) {
    expect(Object.keys(values).sort(), key).toEqual(['de', 'pl', 'uk']);
  }
});
