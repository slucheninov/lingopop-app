import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { listen } from '@tauri-apps/api/event';
import Popup from './Popup.svelte';
import ActionRow from './ActionRow.svelte';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../lib/i18n';
import { isIconOnly, pickMode, popupWidth, rowWidth, panelWidth, viewKey } from '../lib/actionRow';
import { createResizeReporter } from '../lib/resizeReporter';
import LoadingView from './LoadingView.svelte';
import type { ResultModel } from '../lib/types';
import ResultView from './ResultView.svelte';
import ErrorView from './ErrorView.svelte';
import OcrView from './OcrView.svelte';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(vi.fn()) }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ font:'', measureText:(text:string) => ({width:text.length*7}) } as unknown as CanvasRenderingContext2D);
});
const row = { preferred: 'mixed', candidates: ['mixed', 'compact'] } as const;
const result: ResultModel = {
  phase: 'result', text: 'Hello', operation: 'translate', provider: 'OpenAI', providerName: 'OpenAI',
  model: 'm', targetLanguage: 'English', viaFallback: false, fallback: null, truncated: false,
  showReplace: true, showExplain: true, quickLanguages: ['English', 'Ukrainian'],
  actionRow: { preferred: row.preferred, candidates: [...row.candidates] }, retryProviders: [],
};
test('t_substitutes_positional_and_ordered_args', () => {
  expect(t('%2$@ / %1$@ / %@ / %lld', ['one', 'two'])).toBe('two / one / one / two');
  expect(t('%lld characters', [42])).toBe('42 characters');
});
test('action_row_mixed_labels_first_two', () => expect([0,1,2,3].map(i => isIconOnly('mixed', i))).toEqual([false,false,true,true]));
test('action_row_compact_icon_only', () => expect([0,1,2].every(i => isIconOnly('compact', i))).toBe(true));
test('action_row_picks_first_fitting_candidate', () => {
  expect(pickMode(['full','mixed','compact'], m => ({full:701,mixed:700,compact:200})[m])).toBe('mixed');
});
test('popup_width_rule_counts_unicode_scalars', () => {
  expect(popupWidth({phase:'loading'})).toBe(320);
  expect(popupWidth({phase:'error'})).toBe(320);
  expect(popupWidth({phase:'result',text:'x'})).toBe(320);
  expect(popupWidth({phase:'ocr',text:'😀'.repeat(150)})).toBe(350);
  expect(popupWidth({phase:'result',text:'x'.repeat(900)})).toBe(700);
});
test('result_hides_replace_for_screen_translate', () => {
  render(ResultView, {model:{...result,operation:'screenTranslate',showReplace:false}});
  expect(screen.queryByRole('button',{name:'Replace'})).toBeNull();
});
test('result_shows_fallback_disclosure', async () => {
  render(ResultView,{model:{...result,viaFallback:true,fallback:{primaryName:'Claude',message:{key:'Unavailable %@',args:['Claude']},details:[]}}});
  await fireEvent.click(screen.getByText('Why fallback was used'));
  expect(screen.getByText('Unavailable Claude')).toBeTruthy();
});
test('error_shows_retry_menu_when_providers', async () => {
  render(ErrorView,{model:{phase:'error',message:{key:'Oops',args:[]},details:[],retryProviders:[{provider:'Claude',name:'Claude'}]}});
  await fireEvent.click(screen.getByRole('button',{name:'Retry'}));
  await fireEvent.click(screen.getByRole('menuitem',{name:'Retry with Claude'}));
  expect(invoke).toHaveBeenCalledWith('popup_action',{action:{type:'retry',provider:'Claude'}});
});
test('ocr_translate_uses_active_provider', async () => {
  render(OcrView,{model:{phase:'ocr',text:'Hello',translateProviders:[{provider:'Claude',name:'Claude'},{provider:'OpenAI',name:'OpenAI'}],activeTranslateProvider:'OpenAI',actionRow:{preferred:'full',candidates:['full','mixed','compact']},retryProviders:[]}});
  await fireEvent.click(screen.getByRole('button',{name:'Translate'}));
  expect(invoke).toHaveBeenCalledWith('popup_action',{action:{type:'translateOcr',provider:'OpenAI'}});
});
test('clicking_copy_sends_copy_action', async () => {
  render(ResultView,{model:result});
  await fireEvent.click(screen.getByRole('button',{name:'Copy'}));
  expect(invoke).toHaveBeenCalledWith('popup_action',{action:{type:'copy'}});
});
test('language_menu_sends_retranslate', async () => {
  render(ResultView,{model:result});
  await fireEvent.change(screen.getByRole('combobox',{name:'Target language'}),{target:{value:'Ukrainian'}});
  expect(invoke).toHaveBeenCalledWith('popup_action',{action:{type:'retranslate',language:'Ukrainian'}});
});
test('model_text_is_never_html', () => {
  const {container}=render(ResultView,{model:{...result,text:'<img src=x onerror=alert(1)>'}});
  expect(container.querySelector('img')).toBeNull();
  expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy();
});

test('action_row_renders_mixed_and_compact_labels', () => {
  const buttons = ['Copy','Replace','Rewrite','Explain'].map(label => ({label,icon:'copy' as const,action:{type:'copy' as const}}));
  const mixed = render(ActionRow,{row:{preferred:'mixed',candidates:['mixed','compact']},buttons,measure:()=>20});
  expect(screen.getByRole('button',{name:'Copy'}).textContent).toBe('Copy');
  expect(screen.getByRole('button',{name:'Replace'}).textContent).toBe('Replace');
  expect(screen.getByRole('button',{name:'Rewrite'}).textContent).toBe('');
  mixed.unmount();
  render(ActionRow,{row:{preferred:'compact',candidates:['compact']},buttons});
  expect(screen.getAllByRole('button').every(button => button.textContent === '')).toBe(true);
});
test('popup_awaits_listener_before_ready_and_unlistens', async () => {
  let resolve!: (unlisten:()=>void) => void;
  vi.mocked(listen).mockReturnValueOnce(new Promise(r => {resolve=r;}));
  const popup=render(Popup);
  expect(invoke).not.toHaveBeenCalledWith('popup_ready');
  const unlisten=vi.fn(); resolve(unlisten);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('popup_ready'));
  popup.unmount(); await tick();
  expect(unlisten).toHaveBeenCalledOnce();
});
test('popup_unmounted_before_listener_resolves_never_ready', async () => {
  let resolve!: (unlisten:()=>void) => void;
  vi.mocked(listen).mockReturnValueOnce(new Promise(r => {resolve=r;}));
  const popup=render(Popup); popup.unmount();
  const unlisten=vi.fn(); resolve(unlisten); await tick();
  expect(invoke).not.toHaveBeenCalledWith('popup_ready');
  expect(unlisten).toHaveBeenCalledOnce();
});
test('popup_reports_logical_size_and_escape_closes', async () => {
  vi.spyOn(HTMLElement.prototype,'getBoundingClientRect').mockReturnValue({width:350,height:200} as DOMRect);
  render(Popup);
  const handler=vi.mocked(listen).mock.calls.at(-1)![1];
  handler({event:'popup://model',id:1,payload:result});
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('popup_resized',{width:350,height:200}));
  await fireEvent.keyDown(window,{key:'Escape'});
  expect(invoke).toHaveBeenCalledWith('popup_action',{action:{type:'close'}});
});

const fixed = (n: number) => () => n;
const four = ['Copy','Replace','Rewrite','Explain'].map(label => ({label,icon:'copy' as const,action:{type:'copy' as const}}));
test('action_row_mode_follows_available_width', () => {
  // measure=20: full 4*58+32+28=292, mixed 2*58+2*30+32+28=236, compact 4*30+32+28=180.
  const full = ['full','mixed','compact'] as const;
  const a = render(ActionRow,{row:{preferred:'full',candidates:[...full]},buttons:four,measure:fixed(20),available:300});
  expect(a.container.querySelector('.action-row')!.getAttribute('data-mode')).toBe('full');
  a.unmount();
  const b = render(ActionRow,{row:{preferred:'full',candidates:[...full]},buttons:four,measure:fixed(20),available:250});
  expect(b.container.querySelector('.action-row')!.getAttribute('data-mode')).toBe('mixed');
  b.unmount();
  const c = render(ActionRow,{row:{preferred:'full',candidates:[...full]},buttons:four,measure:fixed(20),available:200});
  expect(c.container.querySelector('.action-row')!.getAttribute('data-mode')).toBe('compact');
});
test('ocr_chevron_menu_sends_translate_ocr_with_raw_provider', async () => {
  render(OcrView,{model:{phase:'ocr',text:'Hello',translateProviders:[{provider:'Google (Free)',name:'Google Free'},{provider:'OpenAI',name:'OpenAI'}],activeTranslateProvider:'OpenAI',actionRow:{preferred:'full',candidates:['full','mixed','compact']},retryProviders:[]}});
  const chevron = screen.getByRole('button',{name:'Translate with…'});
  expect(chevron.getAttribute('aria-haspopup')).toBe('menu');
  await fireEvent.click(chevron);
  expect(chevron.getAttribute('aria-controls')).toBe(screen.getByRole('menu',{name:'Translate with…'}).id);
  await fireEvent.click(screen.getByRole('menuitem',{name:'Translate with Google Free'}));
  expect(invoke).toHaveBeenCalledWith('popup_action',{action:{type:'translateOcr',provider:'Google (Free)'}});
});
test('loading_phase_renders_status_and_preview', () => {
  render(LoadingView,{model:{phase:'loading',operation:'grammarFix',preview:'some text',retryProviders:[]}});
  expect(screen.getByRole('status').textContent).toContain('Fixing grammar…');
  expect(screen.getByText('some text')).toBeTruthy();
});
test('popup_renders_loading_phase_without_action_row', async () => {
  render(Popup);
  vi.mocked(listen).mock.calls.at(-1)![1]({event:'popup://model',id:1,payload:{phase:'loading',operation:'translate',preview:'hi',retryProviders:[]}});
  await waitFor(() => expect(screen.getByText('Translating…')).toBeTruthy());
  expect(screen.queryByRole('button',{name:'Copy'})).toBeNull();
});
test('popup_shows_notice_next_to_the_result', async () => {
  render(Popup);
  const handler = vi.mocked(listen).mock.calls.at(-1)![1];
  handler({event:'popup://model',id:1,payload:result});
  await waitFor(() => expect(screen.getByText('Hello')).toBeTruthy());
  expect(screen.queryByRole('status')).toBeNull();
  handler({event:'popup://model',id:2,payload:{...result,notice:{key:'Clipboard busy %@',args:['now']}}});
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Clipboard busy now'));
  expect(screen.getByText('Hello')).toBeTruthy();
  expect(viewKey({...result,notice:{key:'x',args:[]}})).toBe(viewKey(result));
});
const m6 = (text: string) => text.length * 6;
test('row_width_exact_full_arithmetic_matches_core', () => {
  expect(rowWidth(['Copy','Replace'], true, 'full', m6)).toBe(224 + 24 + 28);
});
test('row_width_rounds_text_up_like_core', () => {
  expect(rowWidth(['Rep'], false, 'full', t => t.length * 5.5)).toBe(17 + 38 + 8 + 28);
});
test('row_width_chevron_adds_34_and_empty_is_zero', () => {
  const titles = ['Copy','Translate'];
  expect(rowWidth(titles, true, 'full', m6, 1) - rowWidth(titles, true, 'full', m6, 0)).toBe(14 + 16 + 4);
  expect(rowWidth([], false, 'full', m6)).toBe(0);
});
test('row_width_mode_rules_match_core', () => {
  const en = ['Copy','Replace','Rewrite','Explain'];
  const full = rowWidth(en, true, 'full', m6), mixed = rowWidth(en, true, 'mixed', m6), compact = rowWidth(en, true, 'compact', m6);
  expect(mixed).toBeLessThan(full); expect(compact).toBeLessThan(mixed);
  expect(rowWidth(['Copy','Replace','A much longer rewrite','Explain everything'], true, 'mixed', m6)).toBe(mixed);
  expect(rowWidth(['a','b','c'], true, 'compact', m6)).toBe(rowWidth(['long title','another long','third long'], true, 'compact', m6));
  const t2 = ['Copy','Translate'];
  expect(rowWidth(t2, false, 'mixed', m6, 1)).toBe(rowWidth(t2, false, 'full', m6, 1));
});
test('panel_width_widens_for_the_preferred_row_like_mac', () => {
  // Short text: text width 320, but the preferred-mode row is wider, so the popup grows (plus 2 border px).
  const wide = {...result, actionRow: {preferred:'full' as const, candidates:['full' as const,'mixed' as const,'compact' as const]}};
  const row = rowWidth(['Copy','Replace','Rewrite','Explain'], false, 'full', m6);
  expect(row).toBeGreaterThan(320);
  expect(panelWidth(wide, m6)).toBe(Math.min(700, row + 2));
  expect(panelWidth({phase:'loading',operation:'translate',preview:'',retryProviders:[]}, m6)).toBe(320);
});
test('view_key_ignores_retry_providers_only', () => {
  expect(viewKey({...result,retryProviders:[{provider:'Claude',name:'Claude'}]})).toBe(viewKey(result));
  expect(viewKey({...result,text:'Other'})).not.toBe(viewKey(result));
});
test('resize_reporter_skips_duplicates_and_sends_latest_in_order', async () => {
  const calls: [number, number][] = [];
  const releases: (() => void)[] = [];
  const reporter = createResizeReporter((w, h) => { calls.push([w, h]); return new Promise<void>(r => releases.push(r)); });
  const first = reporter.report(300, 100);
  void reporter.report(300, 120); void reporter.report(300, 140);   // coalesced while the first is in flight
  expect(calls).toEqual([[300, 100]]);
  releases[0]();
  await vi.waitFor(() => expect(calls).toEqual([[300, 100], [300, 140]]));
  releases[1](); await first;
  await reporter.report(300, 140);   // identical to the last sent size
  expect(calls).toEqual([[300, 100], [300, 140]]);
});
test('popup_sends_each_distinct_size_once', async () => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0));
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id));
  vi.spyOn(HTMLElement.prototype,'getBoundingClientRect').mockReturnValue({width:350,height:200} as DOMRect);
  render(Popup);
  const handler=vi.mocked(listen).mock.calls.at(-1)![1];
  handler({event:'popup://model',id:1,payload:result});
  handler({event:'popup://model',id:2,payload:{...result,text:'Hello again'}});
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('popup_resized',{width:350,height:200}));
  await new Promise(r => setTimeout(r, 20));
  expect(vi.mocked(invoke).mock.calls.filter(c => c[0]==='popup_resized')).toHaveLength(1);
});
