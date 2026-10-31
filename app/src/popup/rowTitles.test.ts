import { afterEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import ActionRow from './ActionRow.svelte';
import ResultView from './ResultView.svelte';
import { rowSpec } from '../lib/actionRow';
import type { ResultModel } from '../lib/types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
// Every catalog key is "translated" to a marked string, so a hard-coded English label shows up.
vi.mock('../lib/i18n', () => ({ t: (key: string) => `«${key}»` }));
afterEach(cleanup);

const result: ResultModel = {
  phase: 'result', text: 'Hello', operation: 'translate', provider: 'OpenAI', providerName: 'OpenAI',
  model: 'm', targetLanguage: 'English', viaFallback: false, fallback: null, truncated: false,
  showReplace: true, showExplain: true, quickLanguages: ['English'],
  actionRow: { preferred: 'full', candidates: ['full', 'mixed', 'compact'] },
  retryProviders: [{ provider: 'Claude', name: 'Claude' }],
};

test('row_measurement_uses_the_rendered_titles', () => {
  const measured: string[] = [];
  const spec = rowSpec(result)!;
  const { container } = render(ResultView, { model: result });
  const rendered = [...container.querySelectorAll('.action-row > button:not(.retry), .action-row > .menu > .retry')]
    .map(b => b.getAttribute('aria-label'));
  // Rendered buttons: Copy, Replace, Rewrite, Explain, then Retry.
  expect(rendered).toEqual([...spec.titles, '«Retry»']);
  expect(spec.titles.every(title => title.startsWith('«'))).toBe(true);
  cleanup();
  // The component's own measurement asks for exactly those titles.
  const buttons = spec.titles.map(label => ({ label, icon: 'copy' as const, action: { type: 'copy' as const } }));
  render(ActionRow, { row: result.actionRow, buttons, retryProviders: result.retryProviders, measure: (text: string) => { measured.push(text); return 10; } });
  for (const title of [...spec.titles, '«Retry»']) expect(measured).toContain(title);
  expect(screen.getByRole('button', { name: '«Retry»' })).toBeTruthy();
});
