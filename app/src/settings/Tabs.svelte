<script lang="ts">
  import { icons } from '../lib/icons';
  import { t } from '../lib/i18n';
  import type { TabId } from '../lib/settingsTypes';
  let { active, onSelect }: { active: TabId; onSelect: (tab: TabId) => void } = $props();
  // Mac order. Titles go through t() at render time so the installed catalog applies.
  const TABS: { id: TabId; icon: 'sliders' | 'cpu' | 'command' | 'chart' | 'sparkles' | 'info'; title: () => string }[] = [
    { id: 'main', icon: 'sliders', title: () => t('Main') },
    { id: 'providers', icon: 'cpu', title: () => t('Providers') },
    { id: 'shortcuts', icon: 'command', title: () => t('Shortcuts') },
    { id: 'usage', icon: 'chart', title: () => t('Usage') },
    { id: 'magic', icon: 'sparkles', title: () => t('Magic') },
    { id: 'about', icon: 'info', title: () => t('About') },
  ];
  function onKeydown(event: KeyboardEvent, index: number) {
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = TABS[(index + step + TABS.length) % TABS.length];
    onSelect(next.id);
    document.getElementById(`tab-${next.id}`)?.focus();
  }
</script>
<div class="tabs" role="tablist">
  {#each TABS as tab, index}
    <button type="button" role="tab" id={`tab-${tab.id}`} aria-selected={active === tab.id} aria-controls={`panel-${tab.id}`}
      tabindex={active === tab.id ? 0 : -1} class:active={active === tab.id} onclick={() => onSelect(tab.id)} onkeydown={event => onKeydown(event, index)}>
      {@html icons[tab.icon]}<span>{tab.title()}</span>
    </button>
  {/each}
</div>
<style>
  .tabs { display:flex; justify-content:center; gap:4px; padding:8px 12px 0; border-bottom:1px solid #d5d9df; background:#f3f5f8; }
  button { flex-direction:column; gap:3px; min-width:72px; padding:6px 10px; border:0; border-bottom:2px solid transparent; border-radius:6px 6px 0 0; background:transparent; font-size:12px; color:#596574; }
  button:hover { background:#e6ebf2; }
  button.active { color:#25282c; border-bottom-color:#3478cf; }
</style>
