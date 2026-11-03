<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { t } from '../lib/i18n';
  import { icons } from '../lib/icons';
  import { filterHistory, type HistoryRow as Row } from '../lib/historySearch';
  import HistoryRow from './HistoryRow.svelte';

  let rows = $state<Row[]>([]);
  let query = $state('');
  let error = $state('');
  let readError = $state(false);
  let generation = 0;
  // False until `history://changed` is registered (or after a rejected registration): without the
  // listener a successful remove / clear would never refresh the rows, so those actions reload.
  let listening = false;
  let filtered = $derived(filterHistory(rows, query));

  async function load() {
    const id = ++generation;
    try {
      const next = await invoke<Row[]>('history_list');
      if (id !== generation) return;
      rows = next;
      error = '';
      readError = false;
    } catch {
      if (id === generation) readError = true;
    }
  }

  function actionSucceeded() {
    error = '';
    readError = false;
  }

  // A mutation (remove, clear) changed the store; with a working listener the event refreshes the rows.
  function mutationSucceeded() {
    actionSucceeded();
    if (!listening) void load();
  }

  async function clear() {
    try {
      await invoke('history_clear');
      mutationSucceeded();
    } catch {
      error = t("History couldn't be updated.");
    }
  }

  onMount(() => {
    let disposed = false;
    const registration = listen('history://changed', () => {
      if (!disposed) void load();
    });
    // Listener first, then the initial read: a change between the two is never missed. A failed
    // registration must not block the read.
    void registration.then(
      () => { listening = true; },
      () => { listening = false; },
    ).then(() => {
      if (!disposed) void load();
    });
    return () => {
      disposed = true;
      ++generation;
      void registration.then(remove => remove()).catch(() => {});
    };
  });
</script>
<section>
  <header>
    <input placeholder={t('Search history')} aria-label={t('Search history')} bind:value={query} />
    {#if rows.length}
      <button aria-label={t('Clear')} title={t('Clear')} onclick={clear}>{@html icons.trash}</button>
    {/if}
  </header>
  {#if error}
    <p role="alert">{error}</p>
  {/if}
  {#if readError}
    <p role="status">{t("History couldn't be read.")}</p>
  {/if}
  {#if !rows.length}
    {#if !readError}
      <div class="empty">
        <p>{t('No translations yet.')}</p>
        <small>{t('Press your hotkey on selected text to translate.')}</small>
      </div>
    {/if}
  {:else if !filtered.length}
    <div class="empty">{t('No matches for “%@”.', [query.trim()])}</div>
  {:else}
    <div class="rows">
      {#each filtered as row (row.id)}
        <HistoryRow {row} onerror={message => error = message} onsuccess={actionSucceeded} onmutated={mutationSucceeded} />
      {/each}
    </div>
  {/if}
</section>
<style>
  :global(body) { margin: 0; font-family: system-ui; color: CanvasText; background: Canvas; }
  section { height: 100vh; display: flex; flex-direction: column; }
  header { display: flex; padding: 10px; gap: 10px; border-bottom: 1px solid #8884; }
  input { flex: 1; min-width: 0; padding: 6px; border: 0; background: transparent; color: inherit; }
  .empty { flex: 1; display: flex; flex-direction: column; justify-content: center; align-items: center; color: #777; gap: 10px; }
  .empty p { margin: 0; }
  .rows { overflow: auto; flex: 1; }
</style>
