import { beforeEach, afterEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { filterHistory, formatHistoryDate, type HistoryRow } from '../lib/historySearch';
import History from './History.svelte';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

beforeEach(() => {
  vi.mocked(listen).mockResolvedValue(() => {});
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const row: HistoryRow = {
  id: 'ID',
  original: 'Hello',
  translated: 'Hallo',
  sourceLanguage: 'English',
  targetLanguage: 'German',
  providerName: 'DeepL',
  operation: 'translate',
  operationName: 'Translation',
  timestampUnix: 1700000000,
};

test('filterHistory_rules', () => {
  expect(filterHistory([row], '  HELLO ')).toEqual([row]);
  expect(filterHistory([row], 'hallo')).toEqual([row]);
  expect(filterHistory([row], '')).toEqual([row]);
});

test('formatHistoryDate_uses_language', () => {
  const october = Date.UTC(2026, 9, 15, 12) / 1000;
  const german = formatHistoryDate(october, 'de');
  expect(german).toContain('Okt');
  expect(german).not.toBe(formatHistoryDate(october, 'en'));
});

test('empty_state_texts', async () => {
  vi.mocked(invoke).mockResolvedValue([]);
  render(History);
  await screen.findByText('No translations yet.');
  expect(screen.getByText('Press your hotkey on selected text to translate.')).toBeTruthy();
});

test('no_match_text_has_curly_quotes', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
  await fireEvent.input(screen.getByPlaceholderText('Search history'), { target: { value: 'missing' } });
  expect(screen.getByText('No matches for “missing”.')).toBeTruthy();
});

test('row_menu_copy_delete_and_badge', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('English → German');
  await fireEvent.click(screen.getByRole('button', { name: 'History actions' }));
  await fireEvent.click(screen.getByText('Copy translation'));
  expect(invoke).toHaveBeenCalledWith('clipboard_write', { text: 'Hallo' });
  await fireEvent.click(screen.getByRole('button', { name: 'History actions' }));
  await fireEvent.click(screen.getByText('Delete'));
  expect(invoke).toHaveBeenCalledWith('history_remove', { id: 'ID' });
});

test('history_changed_refetches', async () => {
  vi.mocked(invoke).mockResolvedValue([]);
  render(History);
  await waitFor(() => expect(listen).toHaveBeenCalled());
  const cb = vi.mocked(listen).mock.calls.find(c => c[0] === 'history://changed')![1];
  cb({} as never);
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
});

test('clear_without_confirmation_and_copy_original', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
  await fireEvent.click(screen.getByRole('button', { name: 'History actions' }));
  await fireEvent.click(screen.getByText('Copy original'));
  expect(invoke).toHaveBeenCalledWith('clipboard_write', { text: 'Hello' });
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  expect(invoke).toHaveBeenCalledWith('history_clear');
  expect(screen.queryByRole('dialog')).toBeNull();
});

const changed = () => vi.mocked(listen).mock.calls.find(call => call[0] === 'history://changed')![1]({} as never);

test('failed_read_shows_status_and_successful_refetch_clears_it', async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error('private detail')).mockResolvedValue([row]);
  render(History);
  expect((await screen.findByRole('status')).textContent).toBe("History couldn't be read.");
  changed();
  await screen.findByText('Hallo');
  expect(screen.queryByRole('status')).toBeNull();
});

test('failed_first_read_shows_only_the_read_error', async () => {
  vi.mocked(invoke).mockRejectedValue(new Error('private detail'));
  render(History);
  await screen.findByText("History couldn't be read.");
  expect(screen.queryByText('No translations yet.')).toBeNull();
  expect(screen.queryByText('Press your hotkey on selected text to translate.')).toBeNull();
});

test('row_action_returns_focus_to_trigger', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
  const trigger = screen.getByRole('button', { name: 'History actions' });
  await fireEvent.click(trigger);
  await fireEvent.click(screen.getByRole('button', { name: 'Copy translation' }));
  expect(document.activeElement).toBe(trigger);
});

test('loads_even_if_listener_registration_rejects', async () => {
  vi.mocked(listen).mockRejectedValueOnce(new Error('unavailable'));
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
});

test('successful_clear_clears_previous_action_error', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
  vi.mocked(invoke).mockRejectedValueOnce(new Error('failed'));
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  await screen.findByRole('alert');
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
});

test('row_action_success_clears_previous_error', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
  const trigger = screen.getByRole('button', { name: 'History actions' });
  await fireEvent.click(trigger);
  vi.mocked(invoke).mockRejectedValueOnce(new Error('failed'));
  await fireEvent.click(screen.getByRole('button', { name: 'Copy translation' }));
  await screen.findByRole('alert');
  await fireEvent.click(trigger);
  await fireEvent.click(screen.getByRole('button', { name: 'Copy original' }));
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
});

test('row_menu_accessibility_and_dismissal', async () => {
  vi.mocked(invoke).mockResolvedValue([row]);
  render(History);
  await screen.findByText('Hallo');
  const trigger = screen.getByRole('button', { name: 'History actions' });
  expect(trigger.getAttribute('aria-haspopup')).toBeNull();
  await fireEvent.click(trigger);
  const menu = screen.getByRole('group', { name: 'History actions' });
  expect(trigger.getAttribute('aria-controls')).toBe(menu.id);
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  expect(within(menu).getAllByRole('button')).toHaveLength(3);
  await fireEvent.keyDown(menu, { key: 'Escape' });
  expect(screen.queryByRole('group')).toBeNull();
  expect(document.activeElement).toBe(trigger);
  await fireEvent.click(trigger);
  await fireEvent.click(screen.getByPlaceholderText('Search history'));
  expect(screen.queryByRole('group')).toBeNull();
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
});
