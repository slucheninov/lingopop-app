<script lang="ts">
  import { relaunch } from '../lib/settingsIpc';
  import { t } from '../lib/i18n';
  let { needed }: { needed: boolean } = $props();
  let failure = $state<string | null>(null);
  async function onRelaunch() {
    failure = null;
    try {
      await relaunch();
    } catch (error) {
      // The shell returns a catalog key; an unexpected rejection still shows the generic mac message.
      failure = typeof error === 'string' ? error : 'Could not relaunch LingoPop';
    }
  }
</script>
{#if needed}
  <div class="relaunch">
    <span class="hint">{t('Restart LingoPop to apply the new language.')}</span>
    <button type="button" onclick={onRelaunch}>{t('Relaunch')}</button>
  </div>
  {#if failure}<p class="error" role="alert">{t(failure)}</p>{/if}
{/if}
<style>
  .relaunch { display:flex; align-items:center; justify-content:space-between; gap:8px; }
  .hint { font-size:12px; color:#596574; }
  .error { margin:4px 0 0; font-size:12px; color:#a02c12; }
</style>
