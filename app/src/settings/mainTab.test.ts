import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import Settings from './Settings.svelte';
import { tick } from 'svelte';
import { setLanguage } from '../lib/i18n';
import { baseView, listeners, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); mockIpc(); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

async function openMain() {
  render(Settings);
  await screen.findByRole('switch', { name: 'Auto-detect input language' });
}

test('auto_detect_toggle_sends_patch', async () => {
  await openMain();
  await fireEvent.click(screen.getByRole('switch', { name: 'Auto-detect input language' }));
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'useAutoDetect', value: true } });
});

test('target_select_includes_current_when_not_quick', async () => {
  mockIpc({ settings_get: { ...baseView, targetLanguage: 'Latin' } });
  render(Settings);
  const select = (await screen.findByRole('combobox', { name: 'Target' })) as HTMLSelectElement;
  expect(Array.from(select.options).map(option => option.value)).toEqual(['Latin', 'English', 'Russian', 'Ukrainian', 'Polish']);
  expect(select.value).toBe('Latin');
  await fireEvent.change(select, { target: { value: 'Polish' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'targetLanguage', value: 'Polish' } });
});

test('target_select_label_follows_auto_detect', async () => {
  mockIpc({ settings_get: { ...baseView, useAutoDetect: true } });
  render(Settings);
  expect(await screen.findByRole('combobox', { name: 'Default target' })).toBeTruthy();
});

test('font_slider_sends_ui_patch_and_label', async () => {
  mockIpc({ ui_update: { ...baseView, popupFontSize: 16 } });
  await openMain();
  const slider = screen.getByRole('slider', { name: 'Result font size' }) as HTMLInputElement;
  expect([slider.min, slider.max, slider.step]).toEqual(['11', '20', '1']);
  await fireEvent.input(slider, { target: { value: '16' } });
  expect(invoke).toHaveBeenCalledWith('ui_update', { patch: { field: 'popupFontSize', value: 16 } });
  expect(screen.getByText('16 pt')).toBeTruthy();
});

test('language_select_sends_ui_patch', async () => {
  await openMain();
  const select = screen.getByRole('combobox', { name: 'Language' }) as HTMLSelectElement;
  expect(Array.from(select.options).map(option => option.textContent)).toEqual(['System', 'English', 'Polski', 'Українська', 'Deutsch']);
  await fireEvent.change(select, { target: { value: 'pl' } });
  expect(invoke).toHaveBeenCalledWith('ui_update', { patch: { field: 'appLanguage', value: 'pl' } });
});

test('language_select_shows_relaunch_when_needed', async () => {
  mockIpc({ settings_get: { ...baseView, appLanguage: 'pl', needsRelaunch: true } });
  render(Settings);
  expect(await screen.findByText('Restart LingoPop to apply the new language.')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Relaunch' }));
  expect(invoke).toHaveBeenCalledWith('app_relaunch');
});

test('relaunch_hidden_when_not_needed', async () => {
  await openMain();
  expect(screen.queryByText('Restart LingoPop to apply the new language.')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Relaunch' })).toBeNull();
});

test('relaunch_failure_shows_message', async () => {
  mockIpc({ settings_get: { ...baseView, needsRelaunch: true }, app_relaunch: 'Could not relaunch LingoPop' });
  render(Settings);
  await fireEvent.click(await screen.findByRole('button', { name: 'Relaunch' }));
  expect(await screen.findByText('Could not relaunch LingoPop')).toBeTruthy();
});

test('history_size_sends_patch', async () => {
  await openMain();
  await fireEvent.change(screen.getByRole('spinbutton', { name: 'History size' }), { target: { value: '5' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'maxHistoryCount', value: 5 } });
  await fireEvent.change(screen.getByRole('spinbutton', { name: 'Display' }), { target: { value: '7' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'menuHistoryCount', value: 7 } });
});

test('number_inputs_clamp_and_ignore_garbage', async () => {
  await openMain();
  const history = screen.getByRole('spinbutton', { name: 'History size' }) as HTMLInputElement;
  await fireEvent.change(history, { target: { value: '500' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'maxHistoryCount', value: 100 } });
  expect(history.value).toBe('100');
  vi.mocked(invoke).mockClear();
  await fireEvent.change(screen.getByRole('spinbutton', { name: 'Max input characters' }), { target: { value: '-3' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'maxInputChars', value: 0 } });
});

test('startup_toggles_send_patches', async () => {
  await openMain();
  await fireEvent.click(screen.getByRole('switch', { name: 'Launch at login' }));
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'launchAtLogin', value: true } });
  await fireEvent.click(screen.getByRole('switch', { name: 'Auto-copy OCR/translation results to clipboard' }));
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'autoCopyOCR', value: true } });
});

test('save_failure_shows_message_and_changed_event_refreshes_view', async () => {
  mockIpc({ settings_update: new Error("x") });
  let persisted = baseView;
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command === 'settings_get') return persisted;
    if (command === 'settings_update') throw "Couldn't save settings.";
    return undefined;
  });
  render(Settings);
  await fireEvent.click(await screen.findByRole('switch', { name: 'Launch at login' }));
  expect(await screen.findByRole('alert')).toBeTruthy();
  await waitFor(() => expect(listeners.has('settings://changed')).toBe(true));
  persisted = { ...baseView, targetLanguage: 'Polish' };   // the event is a signal: the view is re-read
  listeners.get('settings://changed')!({ payload: null });
  await waitFor(() => expect((screen.getByRole('combobox', { name: 'Target' }) as HTMLSelectElement).value).toBe('Polish'));
});

test('language_pairs_add_and_remove', async () => {
  const pairsView = { ...baseView, useAutoDetect: true, languagePairs: { Polish: 'English', German: 'English' } };
  mockIpc({ settings_get: pairsView, settings_update: { ...pairsView, languagePairs: { ...pairsView.languagePairs, Russian: 'Ukrainian' } } });
  render(Settings);
  const add = await screen.findByRole('button', { name: 'Add language pair' });
  expect(screen.getByText('If detected source has no pair, the default target is used.')).toBeTruthy();
  const rows = screen.getAllByRole('listitem').map(row => row.textContent?.replace(/\s+/g, ''));
  expect(rows).toEqual(['German→English', 'Polish→English']);
  await fireEvent.click(add);
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'languagePairs', value: { Polish: 'English', German: 'English', Russian: 'Ukrainian' } } });
  const row = (await screen.findAllByRole('listitem'))[0];
  await fireEvent.click(within(row).getByRole('button'));
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'languagePairs', value: { Polish: 'English', Russian: 'Ukrainian' } } });
});

test('language_pairs_hidden_without_auto_detect', async () => {
  await openMain();
  expect(screen.queryByRole('button', { name: 'Add language pair' })).toBeNull();
});

test('font_save_failure_restores_saved_value_and_retry_invokes_again', async () => {
  mockIpc({ ui_update: "Couldn't save settings." });
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command === 'ui_update') throw "Couldn't save settings.";
    return command === 'settings_get' ? baseView : undefined;
  });
  await openMain();
  const slider = screen.getByRole('slider', { name: 'Result font size' }) as HTMLInputElement;
  await fireEvent.input(slider, { target: { value: '16' } });
  await waitFor(() => expect(screen.getByText('13 pt')).toBeTruthy());
  expect(slider.value).toBe('13');
  expect(await screen.findByRole('alert')).toBeTruthy();
  const uiCalls = () => vi.mocked(invoke).mock.calls.filter(call => call[0] === 'ui_update').length;
  expect(uiCalls()).toBe(1);
  await fireEvent.input(slider, { target: { value: '16' } });
  expect(uiCalls()).toBe(2);   // not deduplicated away
});

test('mutations_are_sent_in_order_and_the_last_response_wins', async () => {
  const resolvers: ((view: typeof baseView) => void)[] = [];
  let persisted = baseView;
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command === 'settings_get') return persisted;
    if (command === 'ui_update') return new Promise(resolve => { resolvers.push(resolve as never); });
    return undefined;
  });
  await openMain();
  const slider = screen.getByRole('slider', { name: 'Result font size' }) as HTMLInputElement;
  await fireEvent.input(slider, { target: { value: '14' } });
  await fireEvent.input(slider, { target: { value: '18' } });
  const sent = () => vi.mocked(invoke).mock.calls.filter(call => call[0] === 'ui_update').map(call => (call[1] as { patch: { value: number } }).patch.value);
  expect(sent()).toEqual([14]);   // the second waits for the first to settle
  // A change event for the intermediate commit arrives before the responses.
  listeners.get('settings://changed')!({ payload: { ...baseView, popupFontSize: 14 } });   // a signal: queues one read
  persisted = { ...baseView, popupFontSize: 14 };
  resolvers[0](persisted);
  await waitFor(() => expect(sent()).toEqual([14, 18]));
  expect(screen.getByText('18 pt')).toBeTruthy();   // the stale 14 response did not overwrite the control
  persisted = { ...baseView, popupFontSize: 18 };
  resolvers[1](persisted);
  await waitFor(() => expect(slider.value).toBe('18'));
  expect(screen.getByText('18 pt')).toBeTruthy();
});

test('stale_changed_event_after_the_final_response_does_not_win', async () => {
  const resolvers: ((view: typeof baseView) => void)[] = [];
  let persisted = baseView;
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    if (command === 'settings_get') return persisted;
    if (command === 'ui_update') return new Promise(resolve => { resolvers.push(resolve as never); });
    return undefined;
  });
  await openMain();
  const slider = screen.getByRole('slider', { name: 'Result font size' }) as HTMLInputElement;
  const reads = () => vi.mocked(invoke).mock.calls.filter(call => call[0] === 'settings_get').length;
  await fireEvent.input(slider, { target: { value: '18' } });
  persisted = { ...baseView, popupFontSize: 18 };
  resolvers[0](persisted);
  // Barrier: "18 pt" is already shown optimistically, so wait until the response and its
  // continuations have fully settled before delivering the stale events.
  await new Promise(resolve => setTimeout(resolve, 0));
  await tick();
  expect(screen.getByText('18 pt')).toBeTruthy();
  const before = reads();
  // An event emitted by an EARLIER commit (font 13) is delivered after the final response: payload ignored.
  listeners.get('settings://changed')!({ payload: { ...baseView, popupFontSize: 13 } });
  listeners.get('settings://changed')!({ payload: { ...baseView, popupFontSize: 13 } });
  await waitFor(() => expect(reads()).toBeGreaterThan(before));
  await new Promise(resolve => setTimeout(resolve, 10));
  expect(reads()).toBe(before + 1);   // consecutive signals collapse into one read
  expect(screen.getByText('18 pt')).toBeTruthy();
  expect(slider.value).toBe('18');
});
