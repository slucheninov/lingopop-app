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

  async function clear() {
    try {
      await invoke('history_clear');
      actionSucceeded();
    } catch {
      error = t("History couldn't be updated.");
    }
  }

  onMount(() => {
    let disposed = false;
    const registration = listen('history://changed', () => {
      if (!disposed) void load();
    });
    void registration.catch(() => {});
    void load();
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
    <div class="empty">
      <p>{t('No translations yet.')}</p>
      <small>{t('Press your hotkey on selected text to translate.')}</small>
    </div>
  {:else if !filtered.length}
    <div class="empty">{t('No matches for “%@”.', [query.trim()])}</div>
  {:else}
    <div class="rows">
      {#each filtered as row (row.id)}
        <HistoryRow {row} onerror={message => error = message} onsuccess={actionSucceeded} />
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
