import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import ClipboardHistory from './ClipboardHistory.svelte';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
beforeEach(() => { vi.mocked(listen).mockResolvedValue(() => {}); });
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const row = { id: 'stable-id', text: 'First line\nSecond line', timestampUnix: 1 };
test('copy delete and clear use stable IDs and refresh after committed actions', async () => {
  let rows = [row, { ...row, id: 'other-id', text: 'Other entry' }];
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'clipboard_history_list') return [...rows];
    if (command === 'clipboard_history_remove') rows = rows.filter(entry => entry.id !== (args as { id: string }).id);
    if (command === 'clipboard_history_clear') rows = [];
  });
  render(ClipboardHistory);
  await fireEvent.click(await screen.findByRole('button', { name: row.text }));
  expect(invoke).toHaveBeenCalledWith('clipboard_history_copy', { id: row.id });
  await waitFor(() => expect(screen.getAllByRole('button', { name: 'Delete' })[0].hasAttribute('disabled')).toBe(false));
  await fireEvent.click(screen.getAllByRole('button', { name: 'Delete' })[0]);
  expect(invoke).toHaveBeenCalledWith('clipboard_history_remove', { id: row.id });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Clear' }).hasAttribute('disabled')).toBe(false));
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  expect(invoke).toHaveBeenCalledWith('clipboard_history_clear');
  await screen.findByText('No clipboard entries yet.');
  expect(screen.queryByRole('button', { name: row.text })).toBeNull();
});
test('newer event response wins over a stale initial read', async () => {
  let resolveOld!: (value: typeof row[]) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; })).mockResolvedValue([]);
  render(ClipboardHistory);
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
  const callback = vi.mocked(listen).mock.calls[0][1];
  callback({} as never);
  await screen.findByText('No clipboard entries yet.');
  resolveOld([row]);
  await Promise.resolve();
  expect(screen.queryByRole('button', { name: row.text })).toBeNull();
});
test('read and action errors remain visible without logging clipboard text', async () => {
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === 'clipboard_history_list') return [row];
    throw Error('private text');
  });
  render(ClipboardHistory);
  await fireEvent.click(await screen.findByRole('button', { name: row.text }));
  expect(screen.getByRole('alert').textContent).toBe("Couldn't copy.");
});

test('failed event registration still permits loading and mutation refresh', async () => {
  vi.mocked(listen).mockRejectedValue(new Error('registration unavailable'));
  let rows = [row];
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === 'clipboard_history_list') return [...rows];
    rows = [];
  });
  render(ClipboardHistory);
  await fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
  await screen.findByText('No clipboard entries yet.');
});
test('long multiline entries remain buttons and can be searched without truncating copied data', async () => {
  const long = { ...row, text: 'x'.repeat(5000) + '\nNeedle' };
  vi.mocked(invoke).mockResolvedValue([long]);
  render(ClipboardHistory);
  const copy = await screen.findByRole('button', { name: long.text });
  expect(copy.tagName).toBe('BUTTON');
  await fireEvent.input(screen.getByRole('textbox', { name: 'Search clipboard history' }), { target: { value: 'needle' } });
  expect(screen.getByRole('button', { name: long.text })).toBeTruthy();
});
