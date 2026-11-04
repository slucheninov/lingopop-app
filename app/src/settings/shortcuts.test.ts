import { afterEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { bootstrap } from '../lib/bootstrap';
import { classifyKeydown } from '../lib/hotkeyRecorder';
import Settings from './Settings.svelte';
import { baseView, mockIpc } from './testSupport';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
afterEach(async () => {
  cleanup();
  mockIpc({ ui_bootstrap: { host: 'windows', language: 'en', catalog: {} } });
  await bootstrap();
  vi.clearAllMocks();
});
test('classifyKeydown_rules', () => {
  const e = { code: 'KeyT', ctrlKey: false, altKey: false, shiftKey: false, metaKey: false };
  expect(classifyKeydown({ ...e, ctrlKey: true })).toEqual({ kind: 'record', key: 'KeyT', modifiers: ['ctrl'] });
  expect(classifyKeydown({ ...e, code: 'Escape' })).toEqual({ kind: 'cancel' });
  expect(classifyKeydown(e)).toEqual({ kind: 'ignored' });
  expect(classifyKeydown({ ...e, code: 'ControlLeft', ctrlKey: true })).toEqual({ kind: 'ignored' });
  expect(classifyKeydown({ ...e, code: 'Escape', ctrlKey: true }).kind).toBe('record');
});
async function open(overrides = {}) {
  mockIpc({ settings_get: { ...baseView, hotkeys: { translate: { key: 'KeyT', modifiers: ['ctrl', 'alt'], display: 'Ctrl+Alt+T' } } }, settings_update: baseView, hotkeys_suspend: undefined, ...overrides });
  render(Settings, { initialTab: 'shortcuts' });
  return await screen.findByRole('button', { name: 'Explain' });
}
test('recorder_suspends_while_recording_and_recorded_chord_sends_hotkey_patch', async () => {
  const field = await open();
  await fireEvent.click(field);
  expect(invoke).toHaveBeenCalledWith('hotkeys_suspend', { suspended: true });
  await fireEvent.keyDown(field, { code: 'KeyE', ctrlKey: true, altKey: true });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'hotkey', value: { operation: 'explain', hotkey: { key: 'KeyE', modifiers: ['ctrl', 'alt'] } } } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('hotkeys_suspend', { suspended: false }));
  await fireEvent.click(field); await fireEvent.blur(field);
  await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(c => c[0] === 'hotkeys_suspend' && (c[1] as {suspended: boolean}).suspended === false)).toHaveLength(2));
});
test('clear_button_only_when_set_and_sends_null', async () => {
  await open();
  expect(screen.queryByRole('button', { name: 'Clear Explain' })).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: 'Clear Translate' }));
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'hotkey', value: { operation: 'translate', hotkey: null } } });
});
test('status_lines_rendered', async () => {
  await open({ settings_get: { ...baseView, hotkeyStatus: { translate: 'conflict', explain: 'invalid' } } });
  expect(screen.getByText('This shortcut is already used by another app.')).toBeTruthy();
  expect(screen.getByText("This shortcut can't be registered.")).toBeTruthy();
});
test('duplicate_rejection_renders_inline', async () => {
  const field = await open({ settings_update: () => { throw 'Already used by %@.|Translate'; } });
  await fireEvent.click(field);
  await fireEvent.keyDown(field, { code: 'KeyT', ctrlKey: true, altKey: true });
  expect(await screen.findByText('Already used by Translate.')).toBeTruthy();
});
test('escape_and_unmount_resume', async () => {
  const field = await open(); await fireEvent.click(field);
  await fireEvent.keyDown(field, { code: 'Escape' });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('hotkeys_suspend', { suspended: false }));
  await fireEvent.click(field); cleanup();
  await waitFor(() => expect(vi.mocked(invoke).mock.calls.filter(c => c[0] === 'hotkeys_suspend' && (c[1] as {suspended: boolean}).suspended === false)).toHaveLength(2));
});
test('blur_during_the_suspend_request_resumes_only_after_it_settled', async () => {
  let release: () => void = () => {};
  const gate = new Promise<void>(resolve => { release = resolve; });
  const field = await open({ hotkeys_suspend: (args: { suspended: boolean }) => (args.suspended ? gate : undefined) });
  const suspends = () => vi.mocked(invoke).mock.calls.filter(c => c[0] === 'hotkeys_suspend').map(c => (c[1] as { suspended: boolean }).suspended);
  await fireEvent.click(field);
  await waitFor(() => expect(suspends()).toEqual([true]));
  await fireEvent.blur(field);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(suspends()).toEqual([true]);
  release();
  await waitFor(() => expect(suspends()).toEqual([true, false]));
});
test('unmount_while_the_suspend_request_is_pending_still_resumes_after_it', async () => {
  let release: () => void = () => {};
  const gate = new Promise<void>(resolve => { release = resolve; });
  const field = await open({ hotkeys_suspend: (args: { suspended: boolean }) => (args.suspended ? gate : undefined) });
  const suspends = () => vi.mocked(invoke).mock.calls.filter(c => c[0] === 'hotkeys_suspend').map(c => (c[1] as { suspended: boolean }).suspended);
  await fireEvent.click(field);
  await waitFor(() => expect(suspends()).toEqual([true]));
  cleanup();
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(suspends()).toEqual([true]);
  release();
  await waitFor(() => expect(suspends()).toEqual([true, false]));
});

test('Mac Swift suspension notice and row status are visible', async () => {
  await open({ settings_get: { ...baseView, platform: 'macos', swiftRunning: true, hotkeyStatus: { translate: 'swiftsuspended' } } });
  expect(screen.getByRole('status').textContent).toContain('Quit the Swift LingoPop');
  expect(screen.getByText('Suspended while Swift LingoPop is running.')).toBeTruthy();
});
test('Meta persists as win and Option dead-key text does not replace KeyboardEvent.code', () => {
  const event = { code: 'KeyT', key: 'Dead', ctrlKey: false, altKey: true, shiftKey: false, metaKey: true };
  expect(classifyKeydown(event)).toEqual({ kind: 'record', key: 'KeyT', modifiers: ['alt', 'win'] });
});

test('UiBootstrap host selects the Mac recorder prompt and instructions', async () => {
  mockIpc({ ui_bootstrap: { host: 'unix', language: 'en', catalog: {}, launchedLanguage: 'en', version: '', popupFontSize: 13, initialTab: null } });
  await bootstrap();
  const field = await open();
  expect(screen.getByText(/Use ⌘, Option, Control or Shift/)).toBeTruthy();
  await fireEvent.click(field);
  expect(field.textContent).toBe('Press a key combo (⌘ / Option)…');
  await fireEvent.keyDown(field, { code: 'KeyT', key: 'Dead', altKey: true, metaKey: true });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'hotkey', value: { operation: 'explain', hotkey: { key: 'KeyT', modifiers: ['alt', 'win'] } } } });
});
