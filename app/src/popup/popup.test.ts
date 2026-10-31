import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { listen } from '@tauri-apps/api/event';
import Popup from './Popup.svelte';
import ActionRow from './ActionRow.svelte';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../lib/i18n';
import { isIconOnly, pickMode, popupWidth } from '../lib/actionRow';
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
  await fireEvent.click(screen.getByRole('button',{name:'Retry with Claude'}));
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
