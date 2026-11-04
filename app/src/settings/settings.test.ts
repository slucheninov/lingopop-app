import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import Settings from './Settings.svelte';
import History from '../history/History.svelte';
import { bootstrap } from '../lib/bootstrap';
import { setLanguage, t } from '../lib/i18n';
import { baseBootstrap, listeners, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); mockIpc(); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

test('tabs_render_in_mac_order_and_default_main', async () => {
  render(Settings);
  const tabs = screen.getAllByRole('tab').map(tab => tab.textContent?.trim());
  expect(tabs).toEqual(['Main', 'Providers', 'Shortcuts', 'Usage', 'Magic', 'About']);
  expect(screen.getByRole('tab', { name: 'Main' }).getAttribute('aria-selected')).toBe('true');
  const panel = await screen.findByRole('tabpanel', { name: 'Main' });
  expect(panel).toBeTruthy();
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('settings_get'));
});

test('settings_tab_event_selects_about', async () => {
  render(Settings);
  await waitFor(() => expect(listeners.has('settings://tab')).toBe(true));
  listeners.get('settings://tab')!({ payload: 'about' });
  expect(await screen.findByRole('tabpanel', { name: 'About' })).toBeTruthy();
  expect(screen.getByRole('tab', { name: 'About' }).getAttribute('aria-selected')).toBe('true');
});

test('bootstrap_sets_language_before_mount', async () => {
  mockIpc({ ui_bootstrap: { ...baseBootstrap, language: 'pl', catalog: { Main: 'Główne', About: 'O programie' } } });
  const result = await bootstrap();
  expect(result.language).toBe('pl');
  expect(document.documentElement.lang).toBe('pl');
  render(Settings, { initialTab: result.initialTab });
  expect(t('Main')).toBe('Główne');
  expect(screen.getByRole('tab', { name: 'Główne' })).toBeTruthy();
});

test('bootstrap_initial_tab_opens_about', async () => {
  mockIpc({ ui_bootstrap: { ...baseBootstrap, initialTab: 'about' } });
  const result = await bootstrap();
  render(Settings, { initialTab: result.initialTab });
  expect(await screen.findByRole('tabpanel', { name: 'About' })).toBeTruthy();
});

test('bootstrap_failure_falls_back_to_english', async () => {
  mockIpc({ ui_bootstrap: new Error('boom') });
  const result = await bootstrap();
  expect(result).toEqual({ host: 'windows', language: 'en', launchedLanguage: 'en', catalog: {}, version: '', popupFontSize: 13, initialTab: null });
  expect(t('Close')).toBe('Close');
});

test('history_mounts_and_reads_rows', async () => {
  mockIpc({ history_list: [] });
  render(History);
  expect(screen.getByPlaceholderText('Search history')).toBeTruthy();
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('history_list'));
});
