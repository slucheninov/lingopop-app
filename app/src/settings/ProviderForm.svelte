<script lang="ts">
  import { onMount } from 'svelte';
  import { icons, providerColors, providerIcons } from '../lib/icons';
  import { t } from '../lib/i18n';
  import { resetProviderSecrets, clearProviderKey, getProviderState, openUrl, saveProvider, validateProvider } from '../lib/settingsIpc';
  import type { DraftErrors, ProviderFieldName, ProviderState, SettingsPatch, SettingsView } from '../lib/settingsTypes';
  import type { Provider } from '../lib/types';
  // The parent keys this component by provider, so every unsaved edit (and the key) is dropped on a switch.
  let { provider, view, onPatch }: { provider: Provider; view: SettingsView; onPatch: (patch: SettingsPatch) => Promise<boolean> } = $props();

  let info = $state<ProviderState | null>(null);
  // The API key lives only in this string, bound to the password input. It is read once, by `send`.
  let apiKey = $state('');
  let baseUrl = $state('');
  let model = $state('');
  let referer = $state('');
  let region = $state('');
  let errors = $state<DraftErrors>({ baseUrl: null, referer: null, region: null });
  let failure = $state<string | null>(null);
  let confirmReset = $state(false);
  let cancelReset = $state<HTMLButtonElement | null>(null);
  $effect(() => { if (confirmReset) cancelReset?.focus(); });
  async function resetSecrets() {
    if (saving) return;
    confirmReset = false;
    saving = true;
    failure = null;
    apiKey = '';
    try {
      const next = await resetProviderSecrets(provider);
      if (!disposed) adopt(next);
    } catch { if (!disposed) failure = t("Couldn't reset stored secrets."); }
    finally { if (!disposed) saving = false; }
  }
  let saved = $state(false);
  let saving = $state(false);
  let disposed = false;
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  let validateSeq = 0;

  /** A rejection is a catalog key from the shell; anything else is the generic message. Never log it. */
  const message = (error: unknown) => (typeof error === 'string' ? error : t("Couldn't save settings."));
  const has = (field: ProviderFieldName) => info?.fields.includes(field) ?? false;
  const summary = $derived(view.providers.find(p => p.provider === provider));
  const fallbackChoices = $derived(view.providers.filter(p => p.provider !== provider));
  const isActive = $derived(view.activeProvider === provider);
  const fallback = $derived(summary ? summary.fallback : (info?.fallback ?? null));
  // Saving is refused by the shell while the secrets file is unavailable or its provider settings are unreadable.
  const blocked = $derived(Boolean(info && (info.secretsUnavailable || info.connectionsCorrupt)));
  const invalid = $derived(Boolean(errors.baseUrl || errors.referer || errors.region));
  const resetValue = $derived(info?.autoBaseUrl ? '' : (info?.defaultBaseUrl ?? ''));
  const ready = $derived(info !== null);

  function adopt(next: ProviderState) {
    info = next;
    baseUrl = next.baseUrl; model = next.model; referer = next.referer; region = next.region;
  }

  // Re-validated on every change of the four key-free fields only (the key is not read here), debounced.
  $effect(() => {
    if (!ready) return;
    const fields = { baseUrl, model, referer, region };
    const handle = setTimeout(() => {
      const seq = ++validateSeq;
      validateProvider(provider, fields).then(result => { if (!disposed && seq === validateSeq) errors = result; }).catch(() => {});
    }, 150);
    return () => clearTimeout(handle);
  });

  onMount(() => {
    getProviderState(provider).then(next => { if (!disposed) adopt(next); }).catch(error => { if (!disposed) failure = message(error); });
    return () => { disposed = true; clearTimeout(flashTimer); };
  });

  /** Builds the one request that carries the key; nothing here keeps it. */
  const send = () => saveProvider(provider, { apiKey, baseUrl, model, referer, region });

  async function save() {
    if (!info || blocked || invalid || saving) return;
    saving = true;
    failure = null;
    const request = send();
    // The key leaves the component (info and DOM) before the response is awaited, so a pending or rejected save keeps nothing.
    apiKey = '';
    try {
      const next = await request;
      if (disposed) return;
      adopt(next);
      saved = true;
      clearTimeout(flashTimer);
      flashTimer = setTimeout(() => { saved = false; }, 1500);
    } catch (error) {
      if (!disposed) failure = message(error);
    } finally {
      if (!disposed) saving = false;
    }
  }

  async function clearKey() {
    failure = null;
    try {
      const next = await clearProviderKey(provider);
      if (disposed) return;
      apiKey = '';
      if (info) info.hasKey = next.hasKey;
    } catch (error) {
      if (!disposed) failure = message(error);
    }
  }

  const guide = () => { if (info) void openUrl(info.keyGuideUrl).catch(error => { if (!disposed) failure = message(error); }); };
  const setActive = () => { void onPatch({ field: 'activeProvider', value: provider }); };
  function onFallback(event: Event) {
    const select = event.currentTarget as HTMLSelectElement;
    const value = select.value === '' ? null : (select.value as Provider);
    void onPatch({ field: 'fallback', value: { provider, fallback: value } }).then(ok => { if (!ok) select.value = fallback ?? ''; });
  }
  function pickModel(event: Event) {
    const select = event.currentTarget as HTMLSelectElement;
    if (select.value !== '') model = select.value;
    select.value = '';
  }
</script>
{#snippet errorLine()}{#if failure}<p class="field-error" role="alert">{t(failure)}</p>{/if}{/snippet}
{#if info}
  <form class="form" onsubmit={event => { event.preventDefault(); void save(); }}>
    <section class="header">
      <span class="glyph" style:color={providerColors[provider]}>{@html providerIcons[provider]}</span>
      <div class="titles">
        <h2>{info.name}</h2>
        <span class="ops">{info.operations.map(op => t(op)).join(' · ')}</span>
      </div>
      {#if isActive}
        <span class="active">{@html icons.star}{t('Active')}</span>
      {:else}
        <button type="button" onclick={setActive}>{t('Set as active')}</button>
      {/if}
    </section>

    {#if info.requiresKey}
      <section>
        <h3>{t('API key')}</h3>
        {#if !blocked}
          <div class="row">
            <span class="keystate" class:stored={info.hasKey}>{@html info.hasKey ? icons.check : icons.key}<span>{info.hasKey ? t('A key is stored') : t('No key set')}</span></span>
            {#if info.hasKey}<button type="button" onclick={clearKey}>{t('Clear')}</button>{/if}
          </div>
          <input type="password" autocomplete="off" spellcheck="false" aria-label={t('API key')} bind:value={apiKey}
            placeholder={info.hasKey ? t('Enter a new key to replace') : t('Paste your API key')} />
          {@render errorLine()}
        {/if}
        <p class="caption">{t('Stored locally, encrypted with Windows Data Protection for your user account.')}</p>
        <button type="button" class="link" onclick={guide}>{@html icons.external}<span>{t('How to get an API key')}</span></button>
      </section>
    {:else}
      <section>
        <h3>{t('API key')}</h3>
        <div class="row">
          <span class="keystate stored">{@html icons.check}<span>{t('No key needed')}</span></span>
          <button type="button" class="link" onclick={guide}>{@html icons.external}<span>{t('About this provider (no key needed)')}</span></button>
        </div>
      </section>
    {/if}

    {#if has('baseUrl')}
      <section>
        <h3>{t('Endpoint')}</h3>
        <div class="row">
          <input type="text" class="grow" autocomplete="off" spellcheck="false" aria-label={t('Endpoint')} bind:value={baseUrl}
            placeholder={info.autoBaseUrl ? t('Auto (free/pro chosen by key)') : t('Base URL')} />
          <button type="button" class="icon" aria-label={t('Reset to default endpoint')} title={t('Reset to default endpoint')}
            disabled={baseUrl === resetValue} onclick={() => { baseUrl = resetValue; }}>{@html icons.undo}</button>
        </div>
        <p class="caption">{info.autoBaseUrl
          ? t('Leave blank to choose automatically: keys ending in :fx use api-free.deepl.com, others use %@.', [info.defaultBaseUrl])
          : t('Default: %@', [info.defaultBaseUrl])}</p>
        {#if errors.baseUrl}<p class="field-error" role="alert">{t(errors.baseUrl)}</p>{/if}
      </section>
    {/if}

    {#if has('model') && info.models.length > 0}
      <section>
        <h3>{t('Model')}</h3>
        <div class="row">
          <input type="text" class="grow" autocomplete="off" spellcheck="false" aria-label={t('Model name')} bind:value={model} placeholder={t('Model name')} />
          <select class="presets" aria-label={t('Model')} value="" onchange={pickModel}>
            <option value="">…</option>
            {#each info.models as preset}<option value={preset}>{preset}</option>{/each}
          </select>
        </div>
      </section>
    {/if}

    {#if has('referer')}
      <section>
        <h3>{t('HTTP Referer (optional)')}</h3>
        <input type="text" autocomplete="off" spellcheck="false" aria-label={t('HTTP Referer (optional)')} bind:value={referer} placeholder="https://your-allowed-referer/" />
        <p class="caption">{t("Use this if your API key has 'HTTP referrers' restriction set in Google Cloud Console.")}</p>
        {#if errors.referer}<p class="field-error" role="alert">{t(errors.referer)}</p>{/if}
      </section>
    {/if}

    {#if has('region')}
      <section>
        <h3>{t('Region (optional)')}</h3>
        <input type="text" autocomplete="off" spellcheck="false" aria-label={t('Region (optional)')} bind:value={region} placeholder={t('e.g. eastus')} />
        <p class="caption">{t('Required for multi-service or regional Azure resources. Leave empty for global free-tier (F0) resources.')}</p>
        {#if errors.region}<p class="field-error" role="alert">{t(errors.region)}</p>{/if}
      </section>
    {/if}

    <section>
      <h3>{t('Fallback')}</h3>
      <label class="row"><span>{t('If %@ fails', [info.name])}</span>
        <select value={fallback ?? ''} onchange={onFallback}>
          <option value="">{t("Don't fall back")}</option>
          {#each fallbackChoices as choice}<option value={choice.provider}>{choice.name}</option>{/each}
        </select></label>
      <p class="caption">{t('Triggered on 429 (rate limit), 5xx (server error), and network errors.')}</p>
    </section>

    {#if info.secretsUnavailable}
      <section>
        {#if confirmReset}
          <p>{t('Reset all stored API keys and provider connections? You will need to enter them again.')}</p>
          <div class="row">
            <button type="button" bind:this={cancelReset} onclick={() => { confirmReset = false; }}>{t('Cancel')}</button>
            <button type="button" onclick={resetSecrets} disabled={saving}>{t('Reset stored secrets')}</button>
          </div>
        {:else}
          <button type="button" onclick={() => { confirmReset = true; }} disabled={saving}>{t('Reset stored secrets')}</button>
        {/if}
        {@render errorLine()}
      </section>
    {/if}
    <div class="footer">
      {#if saved}<span class="saved">{@html icons.check}{t('Saved')}</span>{/if}
      {#if blocked}<p class="field-error">{t("Provider settings can't be saved because secrets.dat is unreadable. Remove or rename it and enter your keys again.")}</p>{/if}
      {#if !info.requiresKey}{@render errorLine()}{/if}
      <span class="spacer"></span>
      <button type="submit" class="primary" disabled={blocked || invalid || saving}>{t('Save')}</button>
    </div>
  </form>
{:else}
  {@render errorLine()}
{/if}
<style>
  .form { display:flex; flex-direction:column; gap:10px; }
  section { padding:10px 12px; background:#fff; border:1px solid #e1e5ea; border-radius:8px; display:flex; flex-direction:column; gap:8px; }
  .header { flex-direction:row; align-items:center; gap:10px; }
  .glyph { display:inline-flex; }
  .glyph :global(svg) { width:24px; height:24px; }
  .titles { flex:1; display:flex; flex-direction:column; gap:2px; min-width:0; }
  h2 { margin:0; font-size:16px; font-weight:600; }
  h3 { margin:0; font-size:11px; font-weight:600; text-transform:uppercase; letter-spacing:.04em; color:#596574; }
  .ops { font-size:11px; color:#596574; }
  .active { display:inline-flex; align-items:center; gap:4px; font-size:11px; font-weight:500; color:#b8860b; }
  .active :global(svg) { width:11px; height:11px; }
  .row { display:flex; align-items:center; justify-content:space-between; gap:8px; }
  .keystate { display:inline-flex; align-items:center; gap:6px; font-size:12px; color:#596574; }
  .keystate.stored { color:#2f8a46; }
  input[type='text'], input[type='password'] { width:100%; font:inherit; padding:5px 8px; border:1px solid #cbd0d7; border-radius:5px; background:#fff; }
  .grow { flex:1; min-width:0; }
  .icon { padding:5px 7px; }
  .icon:disabled { opacity:.45; cursor:default; }
  .presets { width:56px; }
  .caption { margin:0; font-size:11px; color:#596574; }
  .field-error { margin:0; font-size:11px; color:#b3261e; }
  .link { align-self:flex-start; border:0; background:transparent; padding:2px 0; font-size:11px; color:#2563b0; }
  .link:hover { background:transparent; text-decoration:underline; }
  .footer { display:flex; align-items:center; gap:10px; }
  .spacer { flex:1; }
  .saved { display:inline-flex; align-items:center; gap:4px; font-size:12px; font-weight:500; color:#2f8a46; }
  .primary { background:#3478cf; border-color:#3478cf; color:#fff; }
  .primary:hover { background:#2a66b3; }
  .primary:disabled { opacity:.5; cursor:default; background:#3478cf; }
</style>
