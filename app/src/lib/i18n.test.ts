import { afterEach, expect, test } from 'vitest';
import { currentLanguage, setLanguage, t } from './i18n';

afterEach(() => setLanguage('en', {}));

test('t_uses_catalog_after_setLanguage', () => {
  expect(currentLanguage()).toBe('en');
  setLanguage('pl', { Copy: 'Kopiuj' });
  expect(currentLanguage()).toBe('pl');
  expect(t('Copy')).toBe('Kopiuj');
  expect(t('Missing')).toBe('Missing');
});

test('t_keeps_substitution_rules', () => {
  expect(t('Retry with %@', ['Claude'])).toBe('Retry with Claude');
  expect(t('Text is %@ characters, max is %@.', [3, 2])).toBe('Text is 3 characters, max is 2.');
  expect(t('%2$@ then %1$@', ['a', 'b'])).toBe('b then a');
  expect(t('Used %lld times', [4])).toBe('Used 4 times');
  expect(t('Missing %@ arg')).toBe('Missing %@ arg');
});

test('t_substitutes_into_the_translated_template', () => {
  setLanguage('de', { 'Retry with %@': 'Erneut mit %@' });
  expect(t('Retry with %@', ['Claude'])).toBe('Erneut mit Claude');
});

test('setLanguage_replaces_the_previous_catalog', () => {
  setLanguage('pl', { Copy: 'Kopiuj' });
  setLanguage('en', {});
  expect(t('Copy')).toBe('Copy');
});
