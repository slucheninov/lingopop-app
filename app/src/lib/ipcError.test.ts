import { expect, test } from 'vitest';
import { renderError } from './ipcError';
import { setLanguage } from './i18n';
test('renderError_splits_key_and_args', () => {
  setLanguage('en', {});
  expect(renderError('Key %@ %@|a|b')).toBe('Key a b');
  expect(renderError('Plain key')).toBe('Plain key');
  setLanguage('pl', { 'Already used by %@.': 'Zajęty przez %@.' });
  expect(renderError('Already used by %@.|Translate')).toBe('Zajęty przez Translate.');
  setLanguage('en', {});
});
