<script lang="ts" module>
  let nextId = 0;
</script>
<script lang="ts">
  import { isIconOnly, pickMode, rowWidth, measureText } from '../lib/actionRow';
  import { icons } from '../lib/icons';
  import { sendAction } from '../lib/ipc';
  import { t } from '../lib/i18n';
  import type { ActionRowView, PopupAction, RetryProvider } from '../lib/types';
  let { row, buttons, retryProviders = [], available = 700, measure = measureText, translateProviders = [] }: {
    row: ActionRowView; buttons: { label: string; icon: keyof typeof icons; action: PopupAction; disabled?: boolean }[];
    retryProviders?: RetryProvider[]; available?: number; measure?: (text: string) => number; translateProviders?: RetryProvider[];
  } = $props();
  const uid = nextId++;
  const translateMenuId = `translate-menu-${uid}`;
  const retryMenuId = `retry-menu-${uid}`;
  let retryOpen = $state(false);
  let translateOpen = $state(false);
  let mode = $derived(pickMode(row.candidates, m => rowWidth(buttons.map(b => b.label), retryProviders.length > 0, m, measure, translateProviders.length ? 1 : 0), Math.min(700, available)));
</script>
<div class="action-row" data-mode={mode}>
  {#each buttons as button, i}
    <button type="button" class:icon-only={isIconOnly(mode, i)} title={button.label} aria-label={button.label} disabled={button.disabled} onclick={() => sendAction(button.action)}>
      {@html icons[button.icon]}{#if !isIconOnly(mode, i)}<span>{button.label}</span>{/if}
    </button>
  {/each}
  {#if translateProviders.length}
    <div class="menu attached">
      <button type="button" title="Translate with…" aria-label="Translate with…" aria-haspopup="menu" aria-expanded={translateOpen} aria-controls={translateOpen ? translateMenuId : undefined} onclick={() => { translateOpen = !translateOpen; retryOpen = false; }}>{@html icons.chevron}</button>
    </div>
  {/if}
  <span class="spacer"></span>
  {#if retryProviders.length}
    <div class="menu">
      <button type="button" class="retry" class:icon-only={isIconOnly(mode, buttons.length)} title="Retry" aria-label="Retry" aria-haspopup="menu" aria-expanded={retryOpen} aria-controls={retryOpen ? retryMenuId : undefined} onclick={() => { retryOpen = !retryOpen; translateOpen = false; }}>
        {@html icons.retry}{#if !isIconOnly(mode, buttons.length)}<span>Retry</span>{/if}{@html icons.chevron}
      </button>
    </div>
  {/if}
</div>
{#if translateOpen}<div class="menu-items" role="menu" id={translateMenuId} aria-label="Translate with…">
  {#each translateProviders as provider}<button type="button" role="menuitem" onclick={() => { translateOpen = false; sendAction({type:'translateOcr',provider:provider.provider}); }}>{t('Translate with %@', [provider.name])}</button>{/each}
</div>{/if}
{#if retryOpen}<div class="menu-items" role="menu" id={retryMenuId} aria-label="Retry">
  {#each retryProviders as provider}<button type="button" role="menuitem" onclick={() => { retryOpen = false; sendAction({type:'retry',provider:provider.provider}); }}>{t('Retry with %@', [provider.name])}</button>{/each}
</div>{/if}
<style>
  .action-row { display:flex; align-items:center; gap:8px; padding-top:12px; }
  .spacer { flex:1; }
  .icon-only { padding-inline:7px; }
  .retry { gap:0; }
  .retry span { margin-left:6px; }
  .attached { margin-left:-4px; }
  .menu { min-width:0; }
  /* Menus expand in flow so ResizeObserver includes them in the native window height. */
  .menu-items { display:flex; flex-direction:column; gap:4px; padding-top:6px; }
</style>
