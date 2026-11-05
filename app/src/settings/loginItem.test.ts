import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import Settings from './Settings.svelte';
import { setLanguage } from '../lib/i18n';
import { baseView, listeners, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); mockIpc(); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

const macView = { ...baseView, platform: 'macos' as const, launchAtLogin: true };
const APPROVAL = 'Allow LingoPop in System Settings > Login Items to start it at login.';

async function open(view: typeof macView) {
  mockIpc({ settings_get: view, permissions_status: [] });
  render(Settings);
  await screen.findByRole('switch', { name: 'Launch at login' });
}

test('no_login_issue_shows_nothing', async () => {
  await open(macView);
  expect(screen.queryByText(APPROVAL)).toBeNull();
  expect(screen.queryByRole('button', { name: 'Open Login Items settings' })).toBeNull();
});

test('requires_approval_shows_the_line_and_the_open_button_that_sends_no_argument', async () => {
  await open({ ...macView, loginItemIssue: 'requiresApproval' });
  expect(screen.getByText(APPROVAL)).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Open Login Items settings' }));
  expect(invoke).toHaveBeenCalledWith('login_items_open_settings');
  expect(vi.mocked(invoke).mock.calls.filter(c => c[0] === 'login_items_open_settings')[0]).toEqual(['login_items_open_settings']);
});

test('unavailable_and_failed_show_their_own_line_without_the_button', async () => {
  await open({ ...macView, loginItemIssue: 'unavailable' });
  expect(screen.getByText(/Applications folder/)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Open Login Items settings' })).toBeNull();
  cleanup();
  await open({ ...macView, loginItemIssue: 'failed' });
  expect(screen.getByText("Couldn't change Launch at login. Try again.")).toBeTruthy();
});

test('a_failed_open_shows_its_catalog_key_in_an_alert', async () => {
  await open({ ...macView, loginItemIssue: 'requiresApproval' });
  mockIpc({ settings_get: { ...macView, loginItemIssue: 'requiresApproval' }, permissions_status: [], login_items_open_settings: "Couldn't open System Settings." });
  await fireEvent.click(screen.getByRole('button', { name: 'Open Login Items settings' }));
  expect((await screen.findByRole('alert')).textContent).toBe("Couldn't open System Settings.");
});

test('the_line_follows_the_settings_changed_signal', async () => {
  let persisted = { ...macView, loginItemIssue: 'requiresApproval' as const } as typeof macView;
  mockIpc({ settings_get: () => persisted, permissions_status: [] });
  render(Settings);
  expect(await screen.findByText(APPROVAL)).toBeTruthy();
  // The user allowed the item: the shell signals and the re-read view carries no issue.
  persisted = macView;
  await waitFor(() => expect(listeners.has('settings://changed')).toBe(true));
  listeners.get('settings://changed')!({ payload: null });
  await waitFor(() => expect(screen.queryByText(APPROVAL)).toBeNull());
});
