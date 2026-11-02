import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { setLanguage } from '../lib/i18n';
import { countScalars, sanitizeMagicPrompt, truncateScalars } from '../lib/scalars';
import Settings from './Settings.svelte';
import { baseView, listeners, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const RELEASES_PAGE_URL = 'https://github.com/slucheninov/lingopop-app/releases/latest';
const PRIVACY_URL = 'https://github.com/slucheninov/lingopop-app/blob/main/PRIVACY.md';
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(call => call[0] === command);

beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

async function openMagic(overrides: Record<string, unknown> = {}) {
  mockIpc(overrides);
  render(Settings, { initialTab: 'magic' });
  return (await screen.findByRole('textbox', { name: 'System prompt' })) as HTMLTextAreaElement;
}

test('scalar_parity_table', () => {
  // Same values as Rust `chars().count()`.
  for (const [text, count] of [
    ['', 0], ['abc', 3], ['a😀b', 3], ['é', 2], ['é', 1], ['\ud800', 1], ['👨‍👩‍👧', 5], ['Привіт', 6],
  ] as [string, number][]) {
    expect(countScalars(text)).toBe(count);
  }
  expect(truncateScalars('a😀b', 2)).toBe('a😀');
  expect(truncateScalars('a😀b', 3)).toBe('a😀b');
  expect(truncateScalars('abc', 0)).toBe('');
  expect(truncateScalars('😀'.repeat(5), 4)).toBe('😀'.repeat(4));
});

test('sanitize_magic_prompt_parity_table', () => {
  // Expected values follow core `sanitize_magic_prompt`: trim Unicode White_Space, drop ASCII controls except \n \t, cut to 2000 scalars.
  const x1999 = 'x'.repeat(1999);
  const cases: [string, string][] = [
    ['  hi  ', 'hi'],
    ['a\tb\nc', 'a\tb\nc'],
    ['\n\ta\n\t', 'a'],
    ['a\u0001b\u0000c\rd\u001fe', 'abcde'],
    ['a\u007fb', 'a\u007fb'],
    ['a\u0085b', 'a\u0085b'],
    ['\u0085x\u0085', 'x'],
    ['\u00a0x\u3000\u2028', 'x'],
    ['\ufeffx', '\ufeffx'],
    ['\u200bx', '\u200bx'],
    [' \u0001x ', 'x'],
    ['é — Привіт', 'é — Привіт'],
    ['\u0001' + x1999 + 'Y', x1999 + 'Y'],
    ['😀'.repeat(2001), '😀'.repeat(2000)],
    [`${'x'.repeat(1999)}😀z`, `${x1999}😀`],
  ];
  for (const [input, want] of cases) expect(sanitizeMagicPrompt(input)).toBe(want);
});

test('live_input_drops_controls_before_cutting', async () => {
  const area = await openMagic();
  await fireEvent.input(area, { target: { value: `\u0001${'x'.repeat(1999)}Y` } });
  expect(area.value).toBe(`${'x'.repeat(1999)}Y`);
  expect(screen.getByText(/ \/ 2000$/).textContent).toBe('2000 / 2000');
});

test('counter_red_at_1900_and_truncates_at_2000', async () => {
  const area = await openMagic();
  const counter = () => screen.getByText(/ \/ 2000$/);
  await fireEvent.input(area, { target: { value: 'x'.repeat(1899) } });
  expect(counter().textContent).toBe('1899 / 2000');
  expect(counter().classList.contains('warn')).toBe(false);
  await fireEvent.input(area, { target: { value: 'x'.repeat(1900) } });
  expect(counter().textContent).toBe('1900 / 2000');
  expect(counter().classList.contains('warn')).toBe(true);
  // Counted in scalars: 1999 ASCII + one emoji (two UTF-16 units) is exactly 2000, so nothing is cut.
  await fireEvent.input(area, { target: { value: `${'x'.repeat(1999)}😀` } });
  expect(countScalars(area.value)).toBe(2000);
  await fireEvent.input(area, { target: { value: `${'y'.repeat(2500)}` } });
  expect(area.value.length).toBe(2000);
  expect(counter().textContent).toBe('2000 / 2000');
  await fireEvent.input(area, { target: { value: `${'😀'.repeat(2100)}` } });
  expect(countScalars(area.value)).toBe(2000);
});

test('reset_requires_confirmation_then_sets_default', async () => {
  const area = await openMagic({ settings_update: { ...baseView, magicPrompt: baseView.defaultMagicPrompt } });
  await fireEvent.input(area, { target: { value: 'mine' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
  const dialog = screen.getByRole('dialog');
  expect(dialog.textContent).toContain('Reset prompt?');
  expect(dialog.textContent).toContain('This will replace your prompt with the default. Continue?');
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Cancel' }));
  expect(area.value).toBe('mine');
  expect(calls('settings_update')).toHaveLength(0);
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(area.value).toBe('mine');
  expect(calls('settings_update')).toHaveLength(0);
  await fireEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(area.value).toBe(baseView.defaultMagicPrompt);
  // Like the Mac app, Reset saves the default at once.
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'magicPrompt', value: baseView.defaultMagicPrompt } }));
});

test('save_disabled_until_changed_then_sends_patch', async () => {
  const area = await openMagic({
    settings_get: { ...baseView, magicPrompt: 'saved' },
    settings_update: { ...baseView, magicPrompt: 'edited' },
  });
  await waitFor(() => expect(area.value).toBe('saved'));
  const save = screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  await fireEvent.input(area, { target: { value: 'edited  ' } });
  expect(save.disabled).toBe(false);
  await fireEvent.click(save);
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'magicPrompt', value: 'edited  ' } });
  // The draft becomes the sanitized saved value and Save is disabled again.
  await waitFor(() => expect(area.value).toBe('edited'));
  expect(save.disabled).toBe(true);
});

test('failed_save_keeps_the_draft', async () => {
  const area = await openMagic({ settings_update: () => { throw "Couldn't save settings."; } });
  await fireEvent.input(area, { target: { value: 'mine' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  expect((await screen.findByRole('alert')).textContent).toContain("Couldn't save settings.");
  expect(area.value).toBe('mine');
  expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(false);
});

async function openAbout(overrides: Record<string, unknown> = {}) {
  mockIpc(overrides);
  render(Settings, { initialTab: 'about' });
  await screen.findByRole('button', { name: 'Check for updates' });
}

test('about_shows_version_and_banner_on_update', async () => {
  await openAbout({ settings_get: { ...baseView, version: '0.2.0', latestVersion: '9.9.9' }, open_url: undefined });
  expect(screen.getByText('LingoPop')).toBeTruthy();
  expect(screen.getByText('Version 0.2.0')).toBeTruthy();
  expect(screen.getByText('Version 9.9.9 available')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: RELEASES_PAGE_URL });
  await fireEvent.click(screen.getByRole('button', { name: 'Privacy Policy' }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: PRIVACY_URL });
  expect(screen.getByText(/^Select text in any app/)).toBeTruthy();
  expect(screen.getByText(/^Supports Claude, OpenAI/)).toBeTruthy();
  const icon = document.querySelector('img') as HTMLImageElement;
  expect([icon.getAttribute('src'), icon.width, icon.height, icon.alt]).toEqual(['/icon.png', 120, 120, '']);
});

test('no_banner_without_an_update', async () => {
  await openAbout();
  expect(screen.queryByText(/available$/)).toBeNull();
  expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
});

test('check_for_updates_invokes_and_stays_silent_on_null', async () => {
  await openAbout({ updates_check: null });
  await fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('updates_check'));
  await waitFor(() => expect((screen.getByRole('button', { name: 'Check for updates' }) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByText(/available$/)).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
});

test('check_for_updates_shows_banner_for_a_found_version', async () => {
  await openAbout({ updates_check: '9.9.9' });
  await fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
  expect(await screen.findByText('Version 9.9.9 available')).toBeTruthy();
});

test('check_for_updates_rejection_is_silent', async () => {
  await openAbout({ updates_check: new Error('boom') });
  await fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
  await waitFor(() => expect((screen.getByRole('button', { name: 'Check for updates' }) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.queryByText(/boom/)).toBeNull();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(r => { resolve = r; });
  return { promise, resolve };
}
const flush = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

test('superseded_save_response_does_not_restore_the_old_prompt', async () => {
  const update = deferred<unknown>();
  const reread = deferred<unknown>();
  let reads = 0;
  const area = await openMagic({
    settings_get: () => (reads++ === 0 ? { ...baseView, magicPrompt: 'old' } : reread.promise),
    settings_update: () => update.promise,
  });
  await waitFor(() => expect(area.value).toBe('old'));
  await fireEvent.input(area, { target: { value: 'new' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  // A change signal queues a read behind the save, so the save's own response is discarded.
  listeners.get('settings://changed')!({ payload: null });
  update.resolve({ ...baseView, magicPrompt: 'new' });
  await waitFor(() => expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true));
  expect(area.value).toBe('new');
  reread.resolve({ ...baseView, magicPrompt: 'new' });
  await waitFor(() => expect(calls('settings_get').length).toBe(2));
  await flush();
  expect(area.value).toBe('new');
  expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true);
});

test('edits_typed_during_a_save_are_kept', async () => {
  const update = deferred<unknown>();
  const area = await openMagic({ settings_get: { ...baseView, magicPrompt: 'old' }, settings_update: () => update.promise });
  await waitFor(() => expect(area.value).toBe('old'));
  await fireEvent.input(area, { target: { value: 'a' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  await fireEvent.input(area, { target: { value: 'ab' } });
  update.resolve({ ...baseView, magicPrompt: 'a' });
  await flush();
  expect(area.value).toBe('ab');
  expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(false);
});

test('clearing_the_text_during_a_save_is_kept', async () => {
  const update = deferred<unknown>();
  const reread = deferred<unknown>();
  let reads = 0;
  const area = await openMagic({
    settings_get: () => (reads++ === 0 ? { ...baseView, magicPrompt: '' } : reread.promise),
    settings_update: () => update.promise,
  });
  await fireEvent.input(area, { target: { value: 'new' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  listeners.get('settings://changed')!({ payload: null });
  // The user clears the field again before the response arrives.
  await fireEvent.input(area, { target: { value: '' } });
  update.resolve({ ...baseView, magicPrompt: 'new' });
  await flush();
  expect(area.value).toBe('');
  reread.resolve({ ...baseView, magicPrompt: 'new' });
  await waitFor(() => expect(calls('settings_get').length).toBe(2));
  await flush();
  expect(area.value).toBe('');
  expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(false);
});

test('clearing_the_text_during_a_save_is_kept_when_the_response_is_the_view', async () => {
  // No queued read: the save response itself becomes the view while the field was cleared.
  const update = deferred<unknown>();
  const area = await openMagic({ settings_get: { ...baseView, magicPrompt: '' }, settings_update: () => update.promise });
  await fireEvent.input(area, { target: { value: 'new' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  await fireEvent.input(area, { target: { value: '' } });
  update.resolve({ ...baseView, magicPrompt: 'new' });
  await flush();
  await waitFor(() => expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(false));
  expect(area.value).toBe('');
});

test('reset_adopts_the_default_after_the_response', async () => {
  const area = await openMagic({ settings_get: { ...baseView, magicPrompt: 'mine' }, settings_update: { ...baseView, magicPrompt: baseView.defaultMagicPrompt } });
  await waitFor(() => expect(area.value).toBe('mine'));
  await fireEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'magicPrompt', value: baseView.defaultMagicPrompt } }));
  await flush();
  expect(area.value).toBe(baseView.defaultMagicPrompt);
  expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true);
  // A later view change now moves the draft again (the reset was adopted, nothing is dirty).
});

test('a_changed_view_updates_an_unedited_draft_only', async () => {
  let current = 'one';
  const area = await openMagic({ settings_get: () => ({ ...baseView, magicPrompt: current }) });
  await waitFor(() => expect(area.value).toBe('one'));
  current = 'two';
  listeners.get('settings://changed')!({ payload: null });
  await waitFor(() => expect(area.value).toBe('two'));
  await fireEvent.input(area, { target: { value: 'mine' } });
  current = 'three';
  listeners.get('settings://changed')!({ payload: null });
  await flush();
  await waitFor(() => expect(calls('settings_get').length).toBeGreaterThan(2));
  expect(area.value).toBe('mine');
});

test('background_update_replaces_an_older_manual_result', async () => {
  let latest: string | null = null;
  await openAbout({ settings_get: () => ({ ...baseView, latestVersion: latest }), updates_check: '9.9.9' });
  await fireEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
  expect(await screen.findByText('Version 9.9.9 available')).toBeTruthy();
  latest = '9.9.10';
  listeners.get('settings://changed')!({ payload: null });
  expect(await screen.findByText('Version 9.9.10 available')).toBeTruthy();
  expect(screen.queryByText('Version 9.9.9 available')).toBeNull();
});
