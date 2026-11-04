import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import Settings from './Settings.svelte';
import { setLanguage } from '../lib/i18n';
import type { ProviderState, ProviderSummary, SettingsView } from '../lib/settingsTypes';
import type { Provider } from '../lib/types';
import { baseView, mockIpc } from './testSupport';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

const ORDER: Provider[] = ['Claude', 'OpenAI', 'Gemini', 'Grok', 'Google (Free)', 'Google Translate', 'DeepL', 'Microsoft Translator'];
const KEYLESS: Provider[] = ['Google (Free)'];
const ANCHORS: Record<Provider, string> = {
  Claude: 'claude', OpenAI: 'openai', Gemini: 'gemini', Grok: 'grok', 'Google (Free)': 'google-free',
  'Google Translate': 'google-translate', DeepL: 'deepl', 'Microsoft Translator': 'microsoft-translator',
};
const GUIDE = 'https://github.com/slucheninov/lingopop-app/blob/main/docs/api-keys.md#';
const NO_ERRORS = { baseUrl: null, referer: null, region: null };

const summaries = (keys: Provider[] = []): ProviderSummary[] =>
  ORDER.map(provider => ({ provider, name: provider, hasKey: keys.includes(provider), requiresKey: !KEYLESS.includes(provider), fallback: null }));
const viewWith = (patch: Partial<SettingsView> = {}, keys: Provider[] = []): SettingsView => ({ ...baseView, providers: summaries(keys), ...patch });

function stateFor(provider: Provider, patch: Partial<ProviderState> = {}): ProviderState {
  const keyless = KEYLESS.includes(provider);
  return {
    provider, name: provider, hasKey: false, requiresKey: !keyless, baseUrl: `https://${ANCHORS[provider]}.example`, model: `${ANCHORS[provider]}-model`,
    referer: '', region: '', defaultBaseUrl: `https://${ANCHORS[provider]}.example`, autoBaseUrl: provider === 'DeepL',
    models: keyless || provider === 'DeepL' ? [] : ['model-a', 'model-b'],
    fields: keyless ? ['baseUrl', 'model'] : provider === 'Microsoft Translator' ? ['apiKey', 'baseUrl', 'region']
      : provider === 'Google Translate' ? ['apiKey', 'baseUrl', 'model', 'referer'] : ['apiKey', 'baseUrl', 'model'],
    operations: ['Translate', 'Screen Translate'], isActive: false, fallback: null, secretsUnavailable: false, secretsLocked: false, connectionsCorrupt: false,
    keyGuideUrl: `${GUIDE}${ANCHORS[provider]}`, ...patch,
  };
}
const providerState = (patch: Partial<ProviderState> = {}) => (args: unknown) => stateFor((args as { provider: Provider }).provider, patch);

function setup(overrides: Record<string, unknown> = {}, view: SettingsView = viewWith({ activeProvider: 'Claude' })) {
  mockIpc({ settings_get: view, settings_update: view, provider_state: providerState(), provider_validate: NO_ERRORS, ...overrides });
}
async function openProviders() {
  render(Settings);
  await fireEvent.click(await screen.findByRole('tab', { name: 'Providers' }));
  await screen.findByRole('listbox', { name: 'Providers' });
}
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(call => call[0] === command);
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const sidebarOptions = () => within(screen.getByRole('listbox', { name: 'Providers' })).getAllByRole('option');
const keyInput = () => screen.getByPlaceholderText('Paste your API key') as HTMLInputElement;
const saveButton = () => screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement;

beforeEach(() => { vi.clearAllMocks(); setLanguage('en', {}); });
afterEach(() => { cleanup(); setLanguage('en', {}); });

test('sidebar_lists_eight_in_order_with_status', async () => {
  setup({}, viewWith({ activeProvider: 'Claude' }, ['OpenAI']));
  await openProviders();
  const options = sidebarOptions();
  expect(options.map(option => within(option).getByText(/^[A-Z]/, { selector: '.name' }).textContent)).toEqual(ORDER);
  expect(within(options[4]).getByText('No key needed')).toBeTruthy();
  expect(within(options[1]).getByText('Key set')).toBeTruthy();
  expect(within(options[0]).getByText('No key')).toBeTruthy();
  expect(within(options[0]).getByLabelText('Active')).toBeTruthy();
  expect(within(options[1]).queryByLabelText('Active')).toBeNull();
});

test('sidebar_shows_the_fallback', async () => {
  const providers = summaries();
  providers[0] = { ...providers[0], fallback: 'OpenAI' };
  providers[1] = { ...providers[1], name: 'OpenAI Display' };
  setup({}, viewWith({ activeProvider: 'Claude', providers }));
  await openProviders();
  expect(within(sidebarOptions()[0]).getByText(/↳ OpenAI Display/)).toBeTruthy();
});

test('selecting_active_by_default', async () => {
  setup({}, viewWith({ activeProvider: 'DeepL' }));
  await openProviders();
  const options = sidebarOptions();
  expect(options.filter(option => option.getAttribute('aria-selected') === 'true').map(option => option.textContent)).toHaveLength(1);
  expect(options[6].getAttribute('aria-selected')).toBe('true');
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('provider_state', { provider: 'DeepL' }));
  expect((await screen.findByRole('heading', { name: 'DeepL' }))).toBeTruthy();
});

test('header_shows_operations_and_active_label', async () => {
  setup();
  await openProviders();
  expect(await screen.findByText('Translate · Screen Translate')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Set as active' })).toBeNull();
  expect(screen.getAllByText('Active').length).toBeGreaterThan(0);
});

test('set_as_active_sends_patch', async () => {
  setup();
  await openProviders();
  await fireEvent.click(sidebarOptions()[1]);
  await fireEvent.click(await screen.findByRole('button', { name: 'Set as active' }));
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'activeProvider', value: 'OpenAI' } });
});

test('save_sends_draft_once_and_clears_key_field', async () => {
  let finish: (state: ProviderState) => void = () => {};
  setup({ provider_save: () => new Promise<ProviderState>(resolve => { finish = resolve; }) });
  await openProviders();
  const input = await screen.findByPlaceholderText('Paste your API key') as HTMLInputElement;
  expect(input.type).toBe('password');
  expect(input.autocomplete).toBe('off');
  await fireEvent.input(input, { target: { value: 'sk-1' } });
  await fireEvent.click(saveButton());
  expect(calls('provider_save')).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith('provider_save', {
    provider: 'Claude',
    draft: { apiKey: 'sk-1', baseUrl: 'https://claude.example', model: 'claude-model', referer: '', region: '' },
  });
  // The save is still pending: the key is already gone from the field and the DOM.
  expect(input.value).toBe('');
  expect(document.body.innerHTML).not.toContain('sk-1');
  finish(stateFor('Claude', { hasKey: true }));
  expect(await screen.findByText('Saved')).toBeTruthy();
  expect(input.value).toBe('');
  expect(calls('provider_save')).toHaveLength(1);
});

test('rejected_save_keeps_the_key_out_and_shows_the_error', async () => {
  setup({ provider_save: () => { throw "Couldn't save the API key. Check that the data folder is writable."; } });
  await openProviders();
  const input = await screen.findByPlaceholderText('Paste your API key') as HTMLInputElement;
  await fireEvent.input(input, { target: { value: 'sk-1' } });
  await fireEvent.click(saveButton());
  expect(await screen.findByText("Couldn't save the API key. Check that the data folder is writable.")).toBeTruthy();
  expect(input.value).toBe('');
  expect(document.body.innerHTML).not.toContain('sk-1');
  expect(screen.queryByText('Saved')).toBeNull();
});

test('rejected_save_refreshes_state_and_keeps_the_error', async () => {
  const error = "Couldn't save the API key. Check that the data folder is writable.";
  let reads = 0;
  setup({
    provider_state: (args: unknown) => {
      reads += 1;
      const provider = (args as { provider: Provider }).provider;
      return stateFor(provider, reads > 1 ? { model: 'committed-model' } : {});
    },
    provider_save: () => { throw error; },
  });
  await openProviders();
  const input = await screen.findByPlaceholderText('Paste your API key') as HTMLInputElement;
  await fireEvent.input(input, { target: { value: 'sk-1' } });
  await fireEvent.click(saveButton());
  await waitFor(() => expect(reads).toBe(2));
  await waitFor(() => expect(screen.getByDisplayValue('committed-model')).toBeTruthy());
  expect(screen.getByText(error)).toBeTruthy();
  expect(input.value).toBe('');
  expect(document.body.innerHTML).not.toContain('sk-1');
});

test('saved_label_goes_away_after_one_and_a_half_seconds', async () => {
  setup({ provider_save: providerState({ hasKey: true }) });
  await openProviders();
  await screen.findByPlaceholderText('Paste your API key');
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  try {
    await fireEvent.click(saveButton());
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByText('Saved')).toBeTruthy();
    await vi.advanceTimersByTimeAsync(1600);
    expect(screen.queryByText('Saved')).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

test('validation_calls_never_carry_the_key', async () => {
  setup();
  await openProviders();
  const input = await screen.findByPlaceholderText('Paste your API key') as HTMLInputElement;
  await waitFor(() => expect(calls('provider_validate').length).toBeGreaterThan(0));
  await sleep(250);
  const settled = calls('provider_validate').length;
  const all = vi.mocked(invoke).mock.calls.length;
  await fireEvent.input(input, { target: { value: 'sk-SENTINEL' } });
  await sleep(300);
  // Typing the key alone triggers no invoke at all.
  expect(calls('provider_validate')).toHaveLength(settled);
  expect(vi.mocked(invoke).mock.calls).toHaveLength(all);
  await fireEvent.input(screen.getByPlaceholderText('Model name'), { target: { value: 'other-model' } });
  await waitFor(() => expect(calls('provider_validate').length).toBe(settled + 1));
  for (const call of calls('provider_validate')) {
    const serialized = JSON.stringify(call[1]);
    expect(serialized).not.toContain('SENTINEL');
    expect(call[1]).toEqual({ provider: 'Claude', fields: expect.any(Object) });
    expect(Object.keys((call[1] as { fields: object }).fields).sort()).toEqual(['baseUrl', 'model', 'referer', 'region']);
  }
  expect(calls('provider_validate').at(-1)![1]).toEqual({
    provider: 'Claude', fields: { baseUrl: 'https://claude.example', model: 'other-model', referer: '', region: '' },
  });
});

test('validation_is_debounced', async () => {
  setup();
  await openProviders();
  const model = await screen.findByPlaceholderText('Model name');
  await waitFor(() => expect(calls('provider_validate').length).toBeGreaterThan(0));
  await sleep(250);
  const before = calls('provider_validate').length;
  for (const value of ['a', 'ab', 'abc']) await fireEvent.input(model, { target: { value } });
  await waitFor(() => expect(calls('provider_validate').length).toBe(before + 1));
  await sleep(300);
  expect(calls('provider_validate')).toHaveLength(before + 1);
});

test('save_disabled_on_validation_error', async () => {
  const key = 'Base URL must start with https://';
  setLanguage('pl', { [key]: 'Adres URL musi zaczynać się od https://' });
  setup({ provider_validate: { ...NO_ERRORS, baseUrl: key } });
  await openProviders();
  expect(await screen.findByText('Adres URL musi zaczynać się od https://')).toBeTruthy();
  expect(saveButton().disabled).toBe(true);
});

test('save_enabled_without_validation_errors', async () => {
  setup();
  await openProviders();
  await screen.findByPlaceholderText('Paste your API key');
  await waitFor(() => expect(saveButton().disabled).toBe(false));
});

test('fallback_select_sends_settings_patch', async () => {
  setup();
  await openProviders();
  const select = await screen.findByRole('combobox', { name: 'If Claude fails' }) as HTMLSelectElement;
  expect(Array.from(select.options).map(option => option.textContent)).toEqual(
    ["Don't fall back", ...ORDER.filter(provider => provider !== 'Claude')]);
  await fireEvent.change(select, { target: { value: 'OpenAI' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'fallback', value: { provider: 'Claude', fallback: 'OpenAI' } } });
  await fireEvent.change(select, { target: { value: '' } });
  expect(invoke).toHaveBeenCalledWith('settings_update', { patch: { field: 'fallback', value: { provider: 'Claude', fallback: null } } });
});

test('secrets_unavailable_disables_key_entry', async () => {
  setup({ provider_state: providerState({ secretsUnavailable: true }) });
  await openProviders();
  expect(await screen.findByText(/secrets\.dat is unreadable/)).toBeTruthy();
  expect(screen.queryByPlaceholderText('Paste your API key')).toBeNull();
  expect(saveButton().disabled).toBe(true);
});

test('secrets_unavailable_blocks_a_keyless_provider_with_the_reason_once', async () => {
  setup({ provider_state: providerState({ secretsUnavailable: true }) }, viewWith({ activeProvider: 'Google (Free)' }));
  await openProviders();
  expect(await screen.findAllByText(/secrets\.dat is unreadable/)).toHaveLength(1);
  expect(saveButton().disabled).toBe(true);
});

test('secrets_unavailable_shows_the_reason_once_for_a_key_provider', async () => {
  setup({ provider_state: providerState({ secretsUnavailable: true }) });
  await openProviders();
  expect(await screen.findAllByText(/secrets\.dat is unreadable/)).toHaveLength(1);
});

test('corrupt_connections_block_save', async () => {
  setup({ provider_state: providerState({ connectionsCorrupt: true }) });
  await openProviders();
  expect(await screen.findByText(/secrets\.dat is unreadable/)).toBeTruthy();
  expect(saveButton().disabled).toBe(true);
});

test('how_to_get_key_opens_allowlisted_url', async () => {
  setup();
  await openProviders();
  await fireEvent.click(sidebarOptions()[6]);
  await fireEvent.click(await screen.findByRole('button', { name: 'How to get an API key' }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: `${GUIDE}deepl` });
});

test('keyless_provider_has_no_key_field', async () => {
  setup({}, viewWith({ activeProvider: 'Google (Free)' }));
  await openProviders();
  expect(await screen.findByText('About this provider (no key needed)')).toBeTruthy();
  expect(screen.getAllByText('No key needed').length).toBeGreaterThan(1);
  expect(screen.queryByPlaceholderText('Paste your API key')).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: 'About this provider (no key needed)' }));
  expect(invoke).toHaveBeenCalledWith('open_url', { url: `${GUIDE}google-free` });
});

test('key_section_reflects_a_stored_key_and_clear_removes_it', async () => {
  let stored = true;
  setup({
    provider_state: (args: unknown) => stateFor((args as { provider: Provider }).provider, { hasKey: stored }),
    provider_clear_key: (args: unknown) => { stored = false; return stateFor((args as { provider: Provider }).provider, { hasKey: false }); },
  });
  await openProviders();
  expect(await screen.findByText('A key is stored')).toBeTruthy();
  expect(screen.getByPlaceholderText('Enter a new key to replace')).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
  expect(invoke).toHaveBeenCalledWith('provider_clear_key', { provider: 'Claude' });
  expect(await screen.findByText('No key set')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Clear' })).toBeNull();
});

test('clear_failure_shows_the_error', async () => {
  setup({
    provider_state: providerState({ hasKey: true }),
    provider_clear_key: () => { throw "Couldn't save the API key. Check that the data folder is writable."; },
  });
  await openProviders();
  await fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
  expect(await screen.findByText("Couldn't save the API key. Check that the data folder is writable.")).toBeTruthy();
  expect(screen.getByText('A key is stored')).toBeTruthy();
});

test('storage_caption_and_endpoint_defaults', async () => {
  setup();
  await openProviders();
  expect(await screen.findByText('Stored locally, encrypted with Windows Data Protection for your user account.')).toBeTruthy();
  expect(screen.getByText('Default: https://claude.example')).toBeTruthy();
  const url = screen.getByPlaceholderText('Base URL') as HTMLInputElement;
  const reset = screen.getByRole('button', { name: 'Reset to default endpoint' }) as HTMLButtonElement;
  expect(reset.disabled).toBe(true);
  await fireEvent.input(url, { target: { value: 'https://elsewhere.example' } });
  expect(reset.disabled).toBe(false);
  await fireEvent.click(reset);
  expect(url.value).toBe('https://claude.example');
});

test('auto_base_url_provider_shows_the_auto_placeholder_and_blank_reset', async () => {
  setup({ provider_state: providerState({ baseUrl: '' }) }, viewWith({ activeProvider: 'DeepL' }));
  await openProviders();
  expect(await screen.findByPlaceholderText('Auto (free/pro chosen by key)')).toBeTruthy();
  expect(screen.getByText('Leave blank to choose automatically: keys ending in :fx use api-free.deepl.com, others use https://deepl.example.')).toBeTruthy();
  expect((screen.getByRole('button', { name: 'Reset to default endpoint' }) as HTMLButtonElement).disabled).toBe(true);
});

test('model_presets_fill_the_model_field', async () => {
  setup();
  await openProviders();
  const menu = await screen.findByRole('combobox', { name: 'Model' }) as HTMLSelectElement;
  await fireEvent.change(menu, { target: { value: 'model-b' } });
  expect((screen.getByPlaceholderText('Model name') as HTMLInputElement).value).toBe('model-b');
});

test('extra_fields_follow_the_provider', async () => {
  setup({}, viewWith({ activeProvider: 'Google Translate' }));
  await openProviders();
  expect(await screen.findByText('HTTP Referer (optional)')).toBeTruthy();
  expect(screen.queryByText('Region (optional)')).toBeNull();
  cleanup();
  setup({}, viewWith({ activeProvider: 'Microsoft Translator' }));
  await openProviders();
  expect(await screen.findByText('Region (optional)')).toBeTruthy();
  expect(screen.getByPlaceholderText('e.g. eastus')).toBeTruthy();
  expect(screen.queryByText('Model')).toBeNull();
  expect(screen.queryByPlaceholderText('Model name')).toBeNull();
});

test('switching_provider_discards_draft', async () => {
  setup();
  await openProviders();
  const input = await screen.findByPlaceholderText('Paste your API key') as HTMLInputElement;
  await fireEvent.input(input, { target: { value: 'sk-DRAFT' } });
  await fireEvent.input(screen.getByPlaceholderText('Model name'), { target: { value: 'edited' } });
  await fireEvent.click(sidebarOptions()[1]);
  await waitFor(() => expect(screen.getByRole('heading', { name: 'OpenAI' })).toBeTruthy());
  expect((await screen.findByPlaceholderText('Model name') as HTMLInputElement).value).toBe('openai-model');
  await fireEvent.click(sidebarOptions()[0]);
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Claude' })).toBeTruthy());
  expect((await screen.findByPlaceholderText('Model name') as HTMLInputElement).value).toBe('claude-model');
  expect(keyInput().value).toBe('');
  expect(document.body.innerHTML).not.toContain('DRAFT');
});


test('unavailable_store_without_lock_does_not_offer_destructive_reset', async () => {
  setup({ provider_state: providerState({ secretsUnavailable: true }) });
  await openProviders();
  await screen.findByText(/secrets\.dat is unreadable/);
  expect(screen.queryByRole('button', { name: 'Reset stored secrets' })).toBeNull();
});


test('locked_store_offers_reset_only_after_confirmation', async () => {
  setup({ provider_state: providerState({ secretsUnavailable: true, secretsLocked: true }), provider_reset_secrets: stateFor('Claude') });
  await openProviders();
  await fireEvent.click(await screen.findByRole('button', { name: 'Reset stored secrets' }));
  expect(calls('provider_reset_secrets')).toHaveLength(0);
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Cancel' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Reset stored secrets' }));
  await waitFor(() => expect(calls('provider_reset_secrets')).toHaveLength(1));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Reset stored secrets' })).toBeNull());
});
