import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import Settings from './Settings.svelte';
import { setLanguage } from '../lib/i18n';
import type { PermissionStatus } from '../lib/settingsTypes';
import { baseView, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(call => call[0] === command);
const RELAUNCH_HINT = 'Screen Recording was granted. Relaunch LingoPop to capture the screen.';
const macView = { ...baseView, platform: 'macos' as const };
const status = (accessibility: PermissionStatus['state'], screenRecording: PermissionStatus['state'], relaunchHint = false): PermissionStatus[] => [
  { kind: 'accessibility', state: accessibility, relaunchHint: false },
  { kind: 'screenRecording', state: screenRecording, relaunchHint },
];

beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

async function openMac(overrides: Record<string, unknown> = {}) {
  mockIpc({ settings_get: macView, permissions_status: status('granted', 'granted'), permissions_refresh: status('granted', 'granted'), ...overrides });
  render(Settings);
  return await screen.findByRole('region', { name: 'Permissions' });
}
const row = (section: HTMLElement, name: string) => within(within(section).getByText(name).closest('[data-permission]') as HTMLElement);

test('section_is_absent_on_windows_and_never_calls_the_permission_commands', async () => {
  mockIpc({ settings_get: { ...baseView, platform: 'windows' } });
  render(Settings);
  await screen.findByRole('switch', { name: 'Auto-detect input language' });
  expect(screen.queryByRole('region', { name: 'Permissions' })).toBeNull();
  expect(calls('permissions_status')).toHaveLength(0);
  expect(calls('permissions_request')).toHaveLength(0);
});

test('mount_reads_the_state_and_never_requests', async () => {
  const section = await openMac({ permissions_status: status('notDetermined', 'denied') });
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Not requested yet')).toBeTruthy());
  expect(row(section, 'Screen Recording').getByText('Denied')).toBeTruthy();
  expect(calls('permissions_status')).toHaveLength(1);
  expect(calls('permissions_request')).toHaveLength(0);
  expect(calls('permissions_open_settings')).toHaveLength(0);
});

test('granted_rows_show_granted_and_no_request_button', async () => {
  const section = await openMac();
  await waitFor(() => expect(within(section).getAllByText('Granted')).toHaveLength(2));
  expect(within(section).queryByRole('button', { name: 'Request' })).toBeNull();
});

test('request_button_asks_only_for_its_own_kind', async () => {
  const section = await openMac({ permissions_status: status('notDetermined', 'notDetermined'), permissions_request: status('notDetermined', 'denied') });
  await waitFor(() => expect(row(section, 'Screen Recording').getByText('Not requested yet')).toBeTruthy());
  await fireEvent.click(row(section, 'Screen Recording').getByRole('button', { name: 'Request' }));
  expect(calls('permissions_request')).toEqual([['permissions_request', { kind: 'screenRecording' }]]);
  await waitFor(() => expect(row(section, 'Screen Recording').getByText('Denied')).toBeTruthy());
});

test('denied_rows_keep_request_and_open_settings_actionable', async () => {
  const section = await openMac({ permissions_status: status('denied', 'denied'), permissions_open_settings: undefined });
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Denied')).toBeTruthy());
  const accessibility = row(section, 'Accessibility');
  expect((accessibility.getByRole('button', { name: 'Request' }) as HTMLButtonElement).disabled).toBe(false);
  await fireEvent.click(accessibility.getByRole('button', { name: 'Open Settings' }));
  // Only the kind id travels; the webview never names a URL.
  expect(calls('permissions_open_settings')).toEqual([['permissions_open_settings', { kind: 'accessibility' }]]);
});

test('restricted_rows_offer_open_settings_but_no_request', async () => {
  const section = await openMac({ permissions_status: status('granted', 'restricted') });
  await waitFor(() => expect(row(section, 'Screen Recording').getByText('Restricted by a profile or administrator')).toBeTruthy());
  const screenRecording = row(section, 'Screen Recording');
  expect(screenRecording.queryByRole('button', { name: 'Request' })).toBeNull();
  expect(screenRecording.getByRole('button', { name: 'Open Settings' })).toBeTruthy();
});

test('returning_to_the_window_refreshes_and_picks_up_a_grant', async () => {
  const section = await openMac({ permissions_status: status('denied', 'granted'), permissions_refresh: status('granted', 'granted') });
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Denied')).toBeTruthy());
  window.dispatchEvent(new Event('focus'));
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Granted')).toBeTruthy());
  expect(calls('permissions_refresh')).toHaveLength(1);
  expect(calls('permissions_request')).toHaveLength(0);
});

test('re_check_button_refreshes', async () => {
  const section = await openMac({ permissions_status: status('denied', 'granted'), permissions_refresh: status('granted', 'granted') });
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Denied')).toBeTruthy());
  await fireEvent.click(within(section).getByRole('button', { name: 'Re-check' }));
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Granted')).toBeTruthy());
});

test('relaunch_hint_shows_with_a_relaunch_button_for_screen_recording_only', async () => {
  const section = await openMac({ permissions_status: status('denied', 'denied', true), app_relaunch: undefined });
  expect(await within(section).findByText(RELAUNCH_HINT)).toBeTruthy();
  expect(within(section).getAllByText(RELAUNCH_HINT)).toHaveLength(1);
  expect(row(section, 'Accessibility').queryByText(RELAUNCH_HINT)).toBeNull();
  await fireEvent.click(within(section).getByRole('button', { name: 'Relaunch' }));
  expect(calls('app_relaunch')).toHaveLength(1);
});

test('no_hint_without_the_flag', async () => {
  const section = await openMac({ permissions_status: status('denied', 'denied') });
  await waitFor(() => expect(row(section, 'Screen Recording').getByText('Denied')).toBeTruthy());
  expect(within(section).queryByText(RELAUNCH_HINT)).toBeNull();
});

test('a_rejected_action_renders_its_catalog_key', async () => {
  const section = await openMac({ permissions_status: status('denied', 'granted'), permissions_open_settings: "Couldn't open System Settings." });
  await waitFor(() => expect(row(section, 'Accessibility').getByText('Denied')).toBeTruthy());
  await fireEvent.click(row(section, 'Accessibility').getByRole('button', { name: 'Open Settings' }));
  expect((await within(section).findByRole('alert')).textContent).toBe("Couldn't open System Settings.");
});

test('focus_listener_is_removed_on_unmount', async () => {
  await openMac();
  cleanup();
  vi.mocked(invoke).mockClear();
  window.dispatchEvent(new Event('focus'));
  expect(calls('permissions_refresh')).toHaveLength(0);
});
