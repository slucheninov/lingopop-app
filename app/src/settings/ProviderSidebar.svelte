<script lang="ts">
  import { icons, providerColors, providerIcons } from '../lib/icons';
  import { t } from '../lib/i18n';
  import type { ProviderSummary } from '../lib/settingsTypes';
  import type { Provider } from '../lib/types';
  let { providers, active, selected, onSelect }: { providers: ProviderSummary[]; active: Provider; selected: Provider; onSelect: (provider: Provider) => void } = $props();
  const nameOf = (provider: Provider) => providers.find(x => x.provider === provider)?.name ?? provider;
  const status = (p: ProviderSummary) => (!p.requiresKey ? t('No key needed') : p.hasKey ? t('Key set') : t('No key'));
  function onKeydown(event: KeyboardEvent, index: number) {
    const step = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const target = (index + step + providers.length) % providers.length;
    onSelect(providers[target].provider);
    document.getElementById(`provider-option-${target}`)?.focus();
  }
</script>
<div class="sidebar" role="listbox" aria-label={t('Providers')}>
  {#each providers as p, index (p.provider)}
    <button type="button" role="option" id={`provider-option-${index}`} aria-selected={selected === p.provider} tabindex={selected === p.provider ? 0 : -1}
      class:selected={selected === p.provider} onclick={() => onSelect(p.provider)} onkeydown={event => onKeydown(event, index)}>
      <span class="glyph" style:color={providerColors[p.provider]}>{@html providerIcons[p.provider]}</span>
      <span class="text">
        <span class="title"><span class="name">{p.name}</span>{#if active === p.provider}<span class="star" role="img" aria-label={t('Active')}>{@html icons.star}</span>{/if}</span>
        <span class="status"><span class:ok={!p.requiresKey || p.hasKey}>{status(p)}</span>{#if p.fallback}<span>· ↳ {nameOf(p.fallback)}</span>{/if}</span>
      </span>
    </button>
  {/each}
</div>
<style>
  .sidebar { display:flex; flex-direction:column; gap:2px; width:220px; flex:none; padding:8px; overflow-y:auto; border-right:1px solid #d5d9df; background:#eef1f5; }
  button { justify-content:flex-start; gap:8px; width:100%; padding:5px 8px; text-align:left; border:0; background:transparent; border-radius:6px; }
  button:hover { background:#e1e7ef; }
  button.selected { background:#cfe0f6; }
  .glyph { display:inline-flex; width:18px; justify-content:center; }
  .text { display:flex; flex-direction:column; gap:1px; min-width:0; }
  .title { display:flex; align-items:center; gap:5px; font-size:13px; font-weight:500; }
  .star { display:inline-flex; color:#e0a800; }
  .star :global(svg) { width:9px; height:9px; }
  .status { display:flex; gap:4px; flex-wrap:wrap; font-size:10px; color:#596574; }
  .status .ok { color:#2f8a46; }
</style>
