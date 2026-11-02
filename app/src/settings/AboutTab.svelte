<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { t } from '../lib/i18n';
  import { checkForUpdates, openUrl, PRIVACY_URL, RELEASES_PAGE_URL } from '../lib/settingsIpc';
  import type { SettingsPatch, SettingsView, UiPatch } from '../lib/settingsTypes';

  // Port of AboutTab.swift. The frame passes the settings props to every tab; About only reads the view.
  let { view }: { view: SettingsView; onPatch?: (patch: SettingsPatch) => Promise<boolean>; onUiPatch?: (patch: UiPatch) => Promise<boolean> } = $props();

  let checking = $state(false);
  let found = $state<string | null>(null);
  let disposed = false;
  // The view only carries a version newer than this build; a lookup made here shows at once.
  let latest = $derived(found ?? view.latestVersion);
  // A fresh view (the check itself and the background loop both signal one) is as new as any manual result, so it wins.
  $effect(() => {
    void view.latestVersion;
    untrack(() => { found = null; });
  });

  onMount(() => () => { disposed = true; });

  /** Silent: no failure message, nothing logged (a rejection may be a catalog key and carries no news). */
  async function check() {
    if (checking) return;
    checking = true;
    try {
      const next = await checkForUpdates();
      if (!disposed && next) found = next;
    } catch {
      // Silent by design.
    } finally {
      if (!disposed) checking = false;
    }
  }

  const open = (url: string) => { void openUrl(url).catch(() => {}); };
</script>
<section class="about">
  <img src="/icon.png" width="120" height="120" alt="" />
  <h2>LingoPop</h2>
  <div class="version">
    <span>{t('Version %@', [view.version])}</span>
    {#if checking}<span class="spinner" role="status" aria-label={t('Check for updates')}></span>{/if}
  </div>
  <button type="button" class="check" disabled={checking} onclick={() => { void check(); }}>{t('Check for updates')}</button>
  {#if latest}
    <div class="banner">
      <span class="label">{t('Version %@ available', [latest])}</span>
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
  .label { flex: 1; text-align: left; font-weight: 500; }
  .download { color: #fff; background: rgba(255, 255, 255, 0.25); border-color: transparent; font-weight: 500; }
  .download:hover { background: rgba(255, 255, 255, 0.4); }
  hr { width: 100%; margin: 4px 0; border: 0; border-top: 1px solid #e1e5ea; }
  p { margin: 0; padding: 0 32px; }
  .detail { font-size: 12px; color: #596574; }
</style>
