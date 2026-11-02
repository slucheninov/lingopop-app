import { tick } from 'svelte';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { setLanguage } from '../lib/i18n';
import type { UsageBucket, UsageView } from '../lib/settingsTypes';
import Settings from './Settings.svelte';
import TokenChart from './TokenChart.svelte';
import { listeners, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const usageView: UsageView = {
  period: 'days30',
  totals: { inputTokens: 0, outputTokens: 0, calls: 0, cost: null },
  costLabel: '~$?',
  tokensLabel: '0',
  inOutLabel: 'in 0 · out 0',
  callsLabel: 'no activity',
  periodLabel: 'last 30 days',
  chartTitle: 'Daily tokens · 30 days',
  buckets: [],
  rows: [],
  unavailable: false,
};

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(call => call[0] === command);

beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

async function openUsage() {
  mockIpc({ usage_get: usageView, usage_reset: undefined });
  render(Settings, { initialTab: 'usage' });
  await screen.findByRole('button', { name: 'Reset stats…' });
}

test('period_default_30_and_switch_refetches', async () => {
  await openUsage();
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('usage_get', { period: 'days30' }));
  await fireEvent.click(screen.getByRole('button', { name: '1 day' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('usage_get', { period: 'day' }));
});

test('chart_stacks_in_tracked_order', () => {
  const bucket: UsageBucket = { startUnix: 1, label: '12', byProvider: { Claude: [1, 1], Grok: [2, 2] } };
  const { container } = render(TokenChart, { buckets: [bucket], label: 'Hourly tokens · today' });
  const rects = [...container.querySelectorAll('svg rect')];
  expect(rects).toHaveLength(2);
  expect(rects[0].getAttribute('data-provider')).toBe('Claude');
  expect(rects[1].getAttribute('data-provider')).toBe('Grok');
  const claudeY = Number(rects[0].getAttribute('y'));
  const grokY = Number(rects[1].getAttribute('y'));
  const claudeH = Number(rects[0].getAttribute('height'));
  const grokH = Number(rects[1].getAttribute('height'));
  expect(claudeY).toBeGreaterThan(grokY);
  expect(grokH / claudeH).toBeCloseTo(2);
});

test('chart_has_text_alternative', () => {
  const buckets: UsageBucket[] = [
    { startUnix: 1, label: '0', byProvider: { Claude: [1, 0] } },
    { startUnix: 2, label: '', byProvider: {} },
  ];
  const { container } = render(TokenChart, { buckets, label: 'Hourly tokens · today' });
  expect(screen.getByRole('img', { name: 'Hourly tokens · today' })).toBeTruthy();
  expect(container.querySelectorAll('table tr')).toHaveLength(buckets.length);
});

test('reset_requires_confirmation', async () => {
  await openUsage();
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  const dialog = screen.getByRole('dialog');
  expect(dialog).toBeTruthy();
  expect(screen.getByText('Delete all usage history?')).toBeTruthy();
  const toolbar = document.querySelector('.toolbar');
  expect(toolbar?.nextElementSibling).toBe(dialog);
  const cancel = screen.getByRole('button', { name: 'Cancel' });
  await waitFor(() => expect(document.activeElement).toBe(cancel));
  await fireEvent.click(cancel);
  expect(calls('usage_reset')).toHaveLength(0);
  expect(screen.queryByRole('dialog')).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
  expect(invoke).toHaveBeenCalledWith('usage_reset');
});

test('cancel_returns_focus_to_reset', async () => {
  await openUsage();
  const reset = screen.getByRole('button', { name: 'Reset stats…' });
  await fireEvent.click(reset);
  await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(document.activeElement).toBe(reset));
});

test('escape_closes_confirm_and_returns_focus_to_reset', async () => {
  await openUsage();
  const reset = screen.getByRole('button', { name: 'Reset stats…' });
  await fireEvent.click(reset);
  await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  expect(screen.queryByRole('dialog')).toBeNull();
  await waitFor(() => expect(document.activeElement).toBe(reset));
  expect(calls('usage_reset')).toHaveLength(0);
});

test('unavailable_store_shows_status_and_disables_reset', async () => {
  mockIpc({ usage_get: { ...usageView, unavailable: true } });
  render(Settings, { initialTab: 'usage' });
  expect(await screen.findByRole('status')).toHaveProperty('textContent', "Usage statistics couldn't be read.");
  expect(screen.getByRole('button', { name: 'Reset stats…' })).toHaveProperty('disabled', true);
});

test('reset_refetches_after_success', async () => {
  await openUsage();
  await waitFor(() => expect(calls('usage_get').length).toBeGreaterThan(0));
  const before = calls('usage_get').length;
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
  await waitFor(() => expect(calls('usage_reset')).toHaveLength(1));
  await waitFor(() => expect(calls('usage_get').length).toBeGreaterThan(before));
});

test('usage_changed_event_refetches', async () => {
  await openUsage();
  await waitFor(() => expect(calls('usage_get').length).toBeGreaterThan(0));
  const before = calls('usage_get').length;
  listeners.get('usage://changed')?.({ payload: null });
  await waitFor(() => expect(calls('usage_get').length).toBeGreaterThan(before));
});

test('rejected_reset_shows_an_alert_and_clears_on_success', async () => {
  let fail = true;
  mockIpc({ usage_get: usageView, usage_reset: () => { if (fail) throw "Usage statistics couldn't be reset."; } });
  render(Settings, { initialTab: 'usage' });
  await screen.findByRole('button', { name: 'Reset stats…' });
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
  expect((await screen.findByRole('alert')).textContent).toBe("Usage statistics couldn't be reset.");
  fail = false;
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
});

test('stale_usage_response_does_not_overwrite_post_reset_data', async () => {
  const old = { ...usageView, tokensLabel: 'OLD' };
  const fresh = { ...usageView, tokensLabel: 'ZERO' };
  let release: (v: UsageView) => void = () => {};
  let finishReset: () => void = () => {};
  let reads = 0;
  mockIpc({
    usage_get: () => {
      reads += 1;
      if (reads === 1) return usageView;
      if (reads === 2) return new Promise<UsageView>(resolve => { release = resolve; });
      return fresh;
    },
    usage_reset: () => new Promise<void>(resolve => { finishReset = resolve; }),
  });
  render(Settings, { initialTab: 'usage' });
  await screen.findByRole('button', { name: 'Reset stats…' });
  await waitFor(() => expect(reads).toBe(1));
  listeners.get('usage://changed')?.({ payload: null });
  await waitFor(() => expect(reads).toBe(2));
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
  await waitFor(() => expect(calls('usage_reset')).toHaveLength(1));
  release(old);
  await tick();
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(screen.queryByText('OLD')).toBeNull();
  finishReset();
  await screen.findByText('ZERO');
  expect(screen.queryByText('OLD')).toBeNull();
});

test('failed_reset_reloads_usage_discarded_by_the_generation_bump', async () => {
  const loaded = { ...usageView, tokensLabel: 'LOADED' };
  let release: (v: UsageView) => void = () => {};
  let reads = 0;
  mockIpc({
    usage_get: () => {
      reads += 1;
      if (reads === 1) return new Promise<UsageView>(resolve => { release = resolve; });
      return loaded;
    },
    usage_reset: () => { throw "Usage statistics couldn't be reset."; },
  });
  render(Settings, { initialTab: 'usage' });
  await screen.findByRole('button', { name: 'Reset stats…' });
  await waitFor(() => expect(reads).toBe(1));
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stats…' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
  expect((await screen.findByRole('alert')).textContent).toBe("Usage statistics couldn't be reset.");
  await waitFor(() => expect(reads).toBe(2));
  await screen.findByText('LOADED');
  release({ ...usageView, tokensLabel: 'STALE' });
  await tick();
  expect(screen.queryByText('STALE')).toBeNull();
});
