<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { icons } from '../lib/icons';
  import { currentLanguage, t } from '../lib/i18n';
  import { formatHistoryDate, type HistoryRow } from '../lib/historySearch';

  let { row, onerror, onsuccess }: {
    row: HistoryRow;
    onerror: (message: string) => void;
    onsuccess: () => void;
  } = $props();
  let menu = $state(false);
  let actions: HTMLDivElement;
  let trigger: HTMLButtonElement;
  let menuId = $derived(`history-menu-${row.id}`);
  const operationIcons: Record<string, string> = {
    translate: icons.translate,
    grammarFix: icons.check,
    rewrite: icons.rewrite,
    explain: icons.explain,
    screenTranslate: icons.translate,
    magic: icons.sparkles,
  };

  function dismiss(event: KeyboardEvent) {
    if (menu && event.key === 'Escape') {
      event.stopPropagation();
      menu = false;
      trigger.focus();
    }
  }

  function outsideClick(event: MouseEvent) {
    if (menu && event.target instanceof Node && !actions.contains(event.target)) menu = false;
  }

  async function act(command: string, args: Record<string, string>) {
    menu = false;
    try {
      await invoke(command, args);
      onsuccess();
    } catch {
      onerror(command === 'clipboard_write' ? t("Couldn't copy.") : t("History couldn't be updated."));
    }
  }
</script>
<svelte:window onclick={outsideClick} onkeydown={dismiss} />
<article>
  <header>
    <span>{@html operationIcons[row.operation ?? ''] ?? icons.translate} {row.operationName}</span>
    {#if row.operation === 'translate'}
      <span class="badge">{row.sourceLanguage ? t('%@ → %@', [row.sourceLanguage, row.targetLanguage]) : row.targetLanguage}</span>
    {/if}
    <span class="badge">{row.providerName}</span>
    <time>{formatHistoryDate(row.timestampUnix, currentLanguage())}</time>
    <div class="actions" bind:this={actions}>
      <button bind:this={trigger} aria-haspopup="menu" aria-label={t('History actions')} aria-controls={menu ? menuId : undefined} aria-expanded={menu} onclick={() => menu = !menu}>…</button>
      {#if menu}
        <div class="menu" role="menu" id={menuId} aria-label={t('History actions')}>
          <button role="menuitem" onclick={() => act('clipboard_write', { text: row.original })}>{t('Copy original')}</button>
          <button role="menuitem" onclick={() => act('clipboard_write', { text: row.translated })}>{t('Copy translation')}</button>
          <button role="menuitem" onclick={() => act('history_remove', { id: row.id })}>{t('Delete')}</button>
        </div>
      {/if}
    </div>
  </header>
  <p class="original">{row.original}</p>
  <p class="translated">{row.translated}</p>
</article>
<style>
  article { padding: 12px 16px; border-bottom: 1px solid #8883; user-select: text; }
  header { display: flex; align-items: center; gap: 6px; color: var(--muted, #777); font-size: 10px; }
  .badge { padding: 2px 6px; background: #8882; border-radius: 4px; }
  time { margin-left: auto; white-space: nowrap; }
  p { white-space: pre-wrap; overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; margin: 6px 0; }
  .original { -webkit-line-clamp: 3; line-clamp: 3; color: #777; font-size: 12px; }
  .translated { -webkit-line-clamp: 6; line-clamp: 6; font-size: 13px; }
  .actions { position: relative; }
  button { cursor: pointer; }
  .menu { position: absolute; right: 0; top: 100%; z-index: 2; min-width: 160px; background: Canvas; box-shadow: 0 2px 12px #0003; padding: 4px; border-radius: 5px; }
  .menu button { display: block; width: 100%; text-align: left; border: 0; background: none; padding: 7px; color: CanvasText; }
</style>
