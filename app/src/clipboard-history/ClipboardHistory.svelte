<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { t } from '../lib/i18n';
  interface Row { id: string; text: string; timestampUnix: number }
  let rows = $state<Row[]>([]);
  let query = $state('');
  let error = $state('');
  let readError = $state(false);
  let generation = 0;
  let disposed = false;
  let busy = $state(false);
  let filtered = $derived(rows.filter(row => row.text.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));
  async function load() {
    const request = ++generation;
    try {
      const next = await invoke<Row[]>('clipboard_history_list');
      if (disposed || request !== generation) return;
      rows = next; readError = false;
    } catch { if (!disposed && request === generation) readError = true; }
  }
  async function action(command: string, id?: string) {
    if (busy) return;
    busy = true;
    // An older read must not repaint content while the mutation is pending.
    ++generation;
    try {
      if (id) await invoke(command, { id }); else await invoke(command);
      error = '';
      await load();
    } catch {
      error = t(command === 'clipboard_history_copy' ? "Couldn't copy." : "Clipboard history couldn't be updated.");
    } finally { busy = false; }
  }
  onMount(() => {
    disposed = false;
    const registration = listen('clipboard-history://changed', () => { if (!disposed) void load(); });
    void registration.catch(() => {}).then(() => { if (!disposed) void load(); });
    return () => { disposed = true; ++generation; void registration.then(unlisten => unlisten()).catch(() => {}); };
  });
</script>
<section>
  <header>
    <input placeholder={t('Search clipboard history')} aria-label={t('Search clipboard history')} bind:value={query} />
    <button disabled={busy || !rows.length} onclick={() => action('clipboard_history_clear')}>{t('Clear')}</button>
  </header>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if readError}<p role="status">{t("Clipboard history couldn't be read.")}</p>{/if}
  <div class="rows">
    {#if !rows.length && !readError}<p class="empty">{t('No clipboard entries yet.')}</p>{/if}
    {#if rows.length && !filtered.length}<p class="empty">{t('No matches for “%@”.', [query.trim()])}</p>{/if}
    {#each filtered as row (row.id)}
      <article>
        <button class="text" disabled={busy} aria-label={row.text} onclick={() => action('clipboard_history_copy', row.id)}>{row.text}</button>
        <button disabled={busy} onclick={() => action('clipboard_history_remove', row.id)}>{t('Delete')}</button>
      </article>
    {/each}
  </div>
</section>
<style>
  :global(body) { margin:0; font-family:system-ui; color:CanvasText; background:Canvas; }
  section { height:100vh; display:flex; flex-direction:column; }
  header { display:flex; gap:12px; padding:12px; border-bottom:1px solid #8884; }
  input { min-width:0; flex:1; padding:6px; color:inherit; background:transparent; border:1px solid #8886; border-radius:5px; }
  button { font:inherit; color:inherit; cursor:pointer; }
  button:focus-visible { outline:2px solid Highlight; outline-offset:2px; }
  .rows { flex:1; overflow:auto; min-height:0; }
  article { display:flex; gap:12px; align-items:flex-start; padding:12px; border-bottom:1px solid #8884; }
  article > button:last-child { flex-shrink:0; }
  .text { min-width:0; flex:1; white-space:pre-wrap; overflow-wrap:anywhere; text-align:left; border:0; padding:4px; background:transparent; max-height:180px; overflow:auto; }
  .text:hover { background:#8881; }
  .empty { text-align:center; padding:24px; color:#777; }
  p[role] { padding:0 12px; }
</style>
