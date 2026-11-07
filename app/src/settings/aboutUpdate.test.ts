import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { setLanguage } from '../lib/i18n';
import type { UpdateView } from '../lib/settingsTypes';
import Settings from './Settings.svelte';
import { baseView, listeners, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const RELEASES_PAGE_URL = 'https://github.com/slucheninov/lingopop-app/releases/latest';
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(call => call[0] === command);
const view = (state: UpdateView['state'], revision: number, extra: Partial<UpdateView> = {}): UpdateView => ({ state, revision, ...extra });

beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

async function openAbout(update: UpdateView, overrides: Record<string, unknown> = {}) {
  mockIpc({ updates_state: update, ...overrides });
  render(Settings, { initialTab: 'about' });
  await screen.findByText('Version 0.2.0');
}

test('available_shows_update_and_whats_new', async () => {
  await openAbout(view('available', 2, { version: '9.9.9' }), { updates_install: view('downloading', 3, { version: '9.9.9', received: 0 }) });
  expect(await screen.findByText('Version 9.9.9 available')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: "What's new" }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: RELEASES_PAGE_URL });
  await fireEvent.click(screen.getByRole('button', { name: 'Update' }));
  await waitFor(() => expect(calls('updates_install')).toHaveLength(1));
  expect(await screen.findByRole('button', { name: 'Cancel' })).toBeTruthy();
});

test('background_banner_checks_before_installing', async () => {
  await openAbout(view('idle', 1), {
    settings_get: { ...baseView, latestVersion: '9.9.9' },
    updates_check: view('available', 2, { version: '9.9.9' }),
    updates_install: view('downloading', 3, { version: '9.9.9', received: 0 }),
  });
  expect(await screen.findByText('Version 9.9.9 available')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Update' }));
  await waitFor(() => expect(calls('updates_install')).toHaveLength(1));
  expect(calls('updates_check')).toHaveLength(1);
});

test('downloading_shows_percent_and_cancel', async () => {
  await openAbout(view('downloading', 3, { version: '9.9.9', received: 2560, total: 10000 }), { updates_cancel: view('idle', 4) });
  expect(await screen.findByText('Downloading… 25%')).toBeTruthy();
  const bar = screen.getByRole('progressbar');
  expect(bar.getAttribute('aria-valuenow')).toBe('25');
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(calls('updates_cancel')).toHaveLength(1));
  await waitFor(() => expect(screen.queryByRole('progressbar')).toBeNull());
});

test('downloading_without_total_is_generic', async () => {
  await openAbout(view('downloading', 3, { version: '9.9.9', received: 5 }));
  expect(await screen.findByText('Downloading…')).toBeTruthy();
});

test('installing_shows_spinner_text', async () => {
  await openAbout(view('installing', 3, { version: '9.9.9' }));
  expect(await screen.findByText('Installing…')).toBeTruthy();
});

test('ready_shows_restart', async () => {
  await openAbout(view('ready', 5, { version: '9.9.9' }), { updates_restart: undefined });
  expect(await screen.findByText('LingoPop 9.9.9 is installed. Restart to finish.')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Restart' }));
  await waitFor(() => expect(calls('updates_restart')).toHaveLength(1));
});

test('failed_shows_try_again_and_download', async () => {
  await openAbout(view('failed', 6, { version: '9.9.9', error: 'The update could not be verified.' }), { updates_install: view('downloading', 7, { version: '9.9.9' }) });
  expect(await screen.findByText('The update could not be verified.')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: RELEASES_PAGE_URL });
  await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
  await waitFor(() => expect(calls('updates_install')).toHaveLength(1));
});

test('unsupported_shows_download_only', async () => {
  await openAbout(view('unsupported', 2, { version: '9.9.9', error: "LingoPop can't update itself here. Download the new version instead." }));
  expect(await screen.findByText('Version 9.9.9 available')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: RELEASES_PAGE_URL });
  expect(screen.queryByRole('button', { name: 'Update' })).toBeNull();
});

test('listener_removed_on_unmount', async () => {
  const unlisten = vi.fn();
  await openAbout(view('idle', 1));
  const original = vi.mocked(listen).getMockImplementation()!;
  cleanup();
  vi.mocked(listen).mockImplementation((async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return name === 'updates://state' ? unlisten : vi.fn();
  }) as unknown as typeof listen);
  render(Settings, { initialTab: 'about' });
  await screen.findByText('Version 0.2.0');
  await waitFor(() => expect(listeners.has('updates://state')).toBe(true));
  await waitFor(() => expect(calls('updates_state').length).toBeGreaterThan(1));
  cleanup();
  expect(unlisten).toHaveBeenCalledTimes(1);
  vi.mocked(listen).mockImplementation(original);
});

test('mount_registers_listener_before_reading_state', async () => {
  const order: string[] = [];
  mockIpc({ updates_state: () => { order.push('read'); return view('ready', 4, { version: '9.9.9' }); } });
  const base = vi.mocked(listen).getMockImplementation()!;
  vi.mocked(listen).mockImplementation((async (name: string, cb: (e: { payload: unknown }) => void) => {
    const unlisten = await (base as (n: string, c: unknown) => Promise<unknown>)(name, cb);
    if (name === 'updates://state') order.push('listened');
    return unlisten;
  }) as unknown as typeof listen);
  render(Settings, { initialTab: 'about' });
  expect(await screen.findByRole('button', { name: 'Restart' })).toBeTruthy();
  expect(order).toEqual(['listened', 'read']);
});

test('stale_read_does_not_overwrite_newer_event', async () => {
  let release!: (v: UpdateView) => void;
  const read = new Promise<UpdateView>(r => { release = r; });
  await openAbout(view('idle', 1), { updates_state: () => read });
  await waitFor(() => expect(listeners.has('updates://state')).toBe(true));
  listeners.get('updates://state')!({ payload: view('ready', 5, { version: '9.9.9' }) });
  expect(await screen.findByRole('button', { name: 'Restart' })).toBeTruthy();
  release(view('downloading', 4, { version: '9.9.9', received: 1, total: 2 }));
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(screen.getByRole('button', { name: 'Restart' })).toBeTruthy();
  expect(screen.queryByRole('progressbar')).toBeNull();
});
