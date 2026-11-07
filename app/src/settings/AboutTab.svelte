<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { t } from '../lib/i18n';
  import { cancelUpdate, checkUpdate, getUpdateState, installUpdate, openUrl, PRIVACY_URL, RELEASES_PAGE_URL, restartForUpdate, UPDATE_EVENT } from '../lib/settingsIpc';
  import type { SettingsPatch, SettingsView, UiPatch, UpdateView } from '../lib/settingsTypes';

  // Port of AboutTab.swift plus the in-app update states (spec section 7). The frame passes the settings props to every tab.
  let { view }: { view: SettingsView; onPatch?: (patch: SettingsPatch) => Promise<boolean>; onUiPatch?: (patch: UiPatch) => Promise<boolean> } = $props();

  let update = $state<UpdateView>({ state: 'idle', revision: 0 });
  let busy = $state(false);
  let restartError = $state<string | null>(null);
  let disposed = false;
  // The background check only sets view.latestVersion; the machine stays idle until a manual check or Update.
  let latest = $derived(view.latestVersion);
  let checking = $derived(busy || update.state === 'checking');

  /** A view older than the one shown is dropped: events, reads and command responses have no ordering guarantee. */
  function apply(next: UpdateView | null | undefined) {
    if (disposed || !next || typeof next.revision !== 'number') return;
    if (next.revision > update.revision) update = next;
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const off = await listen<UpdateView>(UPDATE_EVENT, event => apply(event.payload));
        if (disposed) { off(); return; }
        unlisten = off;
      } catch {
        // Without the event the reads below still show the state.
      }
      try {
        apply(await getUpdateState());
      } catch {
        // Read only; nothing to show.
      }
    })();
    return () => { disposed = true; unlisten?.(); };
  });

  /** Runs one command and applies its response; a rejection is silent (the state machine reports failures itself). */
  async function run(command: () => Promise<UpdateView | void>) {
    if (busy) return;
    busy = true;
    try {
      const next = await command();
      if (next) apply(next);
    } catch {
      // Silent by design.
    } finally {
      if (!disposed) busy = false;
    }
  }

  const check = () => run(checkUpdate);
  const install = () => run(installUpdate);
  /** A banner from the background check: ask the shell for the plan first, then install when it is available. */
  const updateFromBanner = () => run(async () => {
    const checked = await checkUpdate();
    apply(checked);
    return checked.state === 'available' ? installUpdate() : undefined;
  });
  const restart = async () => {
    restartError = null;
    try {
      await restartForUpdate();
    } catch (error) {
      if (!disposed && typeof error === 'string') restartError = error;
    }
  };

  // Every key the shell can send as `error`, as literals so the catalog coverage test sees them.
  const errors = (): Record<string, string> => ({
    'The update could not be verified.': t('The update could not be verified.'),
    'The update could not be downloaded.': t('The update could not be downloaded.'),
    'The update could not be installed. Your current version is unchanged.': t('The update could not be installed. Your current version is unchanged.'),
    "LingoPop can't update itself here. Download the new version instead.": t("LingoPop can't update itself here. Download the new version instead."),
  });
  const errorText = (key: string | undefined) => (key ? (errors()[key] ?? t(key)) : '');
  let percent = $derived(update.total && update.total > 0 ? Math.min(100, Math.floor(((update.received ?? 0) / update.total) * 100)) : null);

  const open = (url: string) => { void openUrl(url).catch(() => {}); };
</script>
<section class="about">
  <img src="/icon.png" width="120" height="120" alt="" />
  <h2>LingoPop</h2>
  <div class="version">
    <span>{t('Version %@', [view.version])}</span>
    {#if checking}<span class="spinner" role="status" aria-label={t('Check for updates')}></span>{/if}
  </div>
  {#if update.state === 'idle' || update.state === 'checking'}
    <button type="button" class="check" disabled={checking} onclick={() => { void check(); }}>{t('Check for updates')}</button>
    {#if latest && update.state === 'idle'}
      <div class="banner">
        <span class="label">{t('Version %@ available', [latest])}</span>
        <button type="button" class="download" disabled={busy} onclick={() => { void updateFromBanner(); }}>{t('Update')}</button>
        <button type="button" class="download" onclick={() => open(RELEASES_PAGE_URL)}>{t("What's new")}</button>
      </div>
    {/if}
  {:else if update.state === 'available'}
    <div class="banner">
      <span class="label">{t('Version %@ available', [update.version ?? ''])}</span>
      <button type="button" class="download" disabled={busy} onclick={() => { void install(); }}>{t('Update')}</button>
      <button type="button" class="download" onclick={() => open(RELEASES_PAGE_URL)}>{t("What's new")}</button>
    </div>
  {:else if update.state === 'downloading'}
    <div class="banner column">
      <span class="label">{percent === null ? t('Downloading…') : t('Downloading… %lld%', [percent])}</span>
      <div class="bar" role="progressbar" aria-label={t('Downloading…')} aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent ?? undefined}>
        <div class="fill" class:indeterminate={percent === null} style:width={percent === null ? '30%' : `${percent}%`}></div>
      </div>
      <button type="button" class="download" disabled={busy} onclick={() => { void run(cancelUpdate); }}>{t('Cancel')}</button>
    </div>
  {:else if update.state === 'installing'}
    <div class="banner">
      <span class="spinner light" aria-hidden="true"></span>
      <span class="label" role="status">{t('Installing…')}</span>
    </div>
  {:else if update.state === 'ready'}
    <div class="banner">
      <span class="label">{t('LingoPop %@ is installed. Restart to finish.', [update.version ?? ''])}</span>
      <button type="button" class="download" onclick={() => { void restart(); }}>{t('Restart')}</button>
    </div>
    {#if restartError}<p class="error" role="alert">{t(restartError)}</p>{/if}
  {:else if update.state === 'failed'}
    <div class="banner failed">
      <span class="label" role="alert">{errorText(update.error)}</span>
      <button type="button" class="download" disabled={busy} onclick={() => { void install(); }}>{t('Try again')}</button>
      <button type="button" class="download" onclick={() => open(RELEASES_PAGE_URL)}>{t('Download')}</button>
    </div>
  {:else if update.state === 'unsupported'}
    <div class="banner">
      <span class="label">{t('Version %@ available', [update.version ?? latest ?? ''])}</span>
      <button type="button" class="download" onclick={() => open(RELEASES_PAGE_URL)}>{t('Download')}</button>
    </div>
  {/if}
  <hr />
  <p class="lead">{t('Select text in any app, press a shortcut, and get an instant AI translation, grammar fix, or rewrite in a popup near your cursor.')}</p>
  <p class="detail">{t('Supports Claude, OpenAI, Gemini, Grok, Google Translate, DeepL, and Microsoft Translator. API keys and provider settings are stored locally, encrypted for your Windows account.')}</p>
  <button type="button" class="link" onclick={() => open(PRIVACY_URL)}>{t('Privacy Policy')}</button>
</section>
<style>
  .about { display: flex; flex-direction: column; align-items: center; gap: 12px; padding-top: 8px; text-align: center; }
  img { object-fit: contain; }
  h2 { margin: 0; font-size: 26px; font-weight: 600; }
  .version { display: flex; align-items: center; gap: 6px; font-size: 11px; color: #596574; }
  .spinner { width: 12px; height: 12px; border: 2px solid #cbd0d7; border-top-color: #3478cf; border-radius: 50%; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
  .check, .link { border: 0; background: none; font-size: 11px; color: #596574; }
  .link { font-size: 12px; color: #3478cf; }
  .banner { display: flex; align-items: center; gap: 10px; width: calc(100% - 64px); padding: 10px 14px; color: #fff; background: #3478cf; border-radius: 10px; }
  .banner.column { flex-direction: column; align-items: stretch; }
  .banner.failed { background: #b3412f; }
  .bar { height: 6px; background: rgba(255, 255, 255, 0.3); border-radius: 3px; overflow: hidden; }
  .fill { height: 100%; background: #fff; }
  .spinner.light { border-color: rgba(255, 255, 255, 0.4); border-top-color: #fff; }
  .error { color: #b3412f; font-size: 12px; }
  .label { flex: 1; text-align: left; font-weight: 500; }
  .download { color: #fff; background: rgba(255, 255, 255, 0.25); border-color: transparent; font-weight: 500; }
  .download:hover { background: rgba(255, 255, 255, 0.4); }
  hr { width: 100%; margin: 4px 0; border: 0; border-top: 1px solid #e1e5ea; }
  p { margin: 0; padding: 0 32px; }
  .detail { font-size: 12px; color: #596574; }
</style>
