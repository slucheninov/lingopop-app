<script lang="ts">
  import { t } from '../lib/i18n';
  import { renderError } from '../lib/ipcError';
  import { onMount } from 'svelte';
  import { openLoginItemsSettings, recheckLoginItem } from '../lib/settingsIpc';
  import type { LoginItemIssue } from '../lib/settingsTypes';

  let { issue }: { issue: LoginItemIssue } = $props();
  let failure = $state<string | null>(null);

  // Back from System Settings after allowing the item: ask the shell to read the state again
  // (read only). The answer is not applied here: when the notice should go, the shell emits
  // `settings://changed` and Settings re-reads the view, so no response can be stale. Calls do
  // not overlap, and only a pending approval has anything to clear.
  let rechecking = false;
  async function recheck() {
    if (rechecking || issue !== 'requiresApproval') return;
    rechecking = true;
    try { await recheckLoginItem(); } catch { /* the next focus tries again */ } finally { rechecking = false; }
  }
  onMount(() => {
    const onFocus = () => { void recheck(); };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  });

  // No argument travels: the shell opens the fixed Login Items pane itself.
  async function openSettings() {
    try { await openLoginItemsSettings(); failure = null; }
    catch (error) { failure = typeof error === 'string' ? error : "Couldn't open System Settings."; }
  }
</script>
<div class="notice">
  {#if issue === 'requiresApproval'}
    <span role="status">{t('Allow LingoPop in System Settings > Login Items to start it at login.')}</span>
    <button type="button" onclick={openSettings}>{t('Open Login Items settings')}</button>
  {:else if issue === 'unavailable'}
    <span role="status">{t("Launch at login isn't available until LingoPop runs from the Applications folder. Move it there and open it again.")}</span>
  {:else if issue === 'serviceNotFound'}
    <span role="status">{t("Launch at login isn't available for this copy of LingoPop. Reinstall it in the Applications folder and open it again.")}</span>
  {:else}
    <span role="status">{t("Couldn't change Launch at login. Try again.")}</span>
  {/if}
</div>
{#if failure}<p class="error" role="alert">{renderError(failure)}</p>{/if}
<style>
  .notice { display:flex; align-items:center; justify-content:space-between; gap:8px; font-size:12px; color:#596574; }
  .error { margin:0; font-size:12px; color:#a02c12; }
</style>
