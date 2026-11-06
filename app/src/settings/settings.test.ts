import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import Settings from './Settings.svelte';
import History from '../history/History.svelte';
import { bootstrap } from '../lib/bootstrap';
import { setLanguage, t } from '../lib/i18n';
import { baseBootstrap, baseView, listeners, mockIpc } from './testSupport';

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

test('settings_reads_again_once_the_change_listener_is_registered', async () => {
  // A change committed after the first read but before the listener existed would otherwise be missed.
  let register!: () => void;
  const registered = new Promise<void>(resolve => { register = resolve; });
  const { listen } = await import('@tauri-apps/api/event');
  // The second read sees the change; the view must show it.
  let served = 0;
  mockIpc({ settings_get: () => (served++ === 0 ? baseView : { ...baseView, maxHistoryCount: 77 }) });
  const original = vi.mocked(listen).getMockImplementation()!;
  vi.mocked(listen).mockImplementation((async (name: string, callback: (event: { payload: unknown }) => void) => {
    if (name === 'settings://changed') await registered;
    return original(name as never, callback as never);
  }) as unknown as typeof listen);
  const reads = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === 'settings_get').length;
  render(Settings);
  await waitFor(() => expect(reads()).toBe(1));
  expect(await screen.findByDisplayValue(String(baseView.maxHistoryCount))).toBeTruthy();
  register();
  await waitFor(() => expect(reads()).toBe(2));
  expect(await screen.findByDisplayValue('77')).toBeTruthy();
});
