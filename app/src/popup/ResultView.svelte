<script lang="ts">
  import ActionRow from './ActionRow.svelte';
  import { t } from '../lib/i18n';
  import { actionTitles } from '../lib/actionRow';
  import { sendAction } from '../lib/ipc';
  import { icons, providerColors, providerIcons } from '../lib/icons';
  import type { ResultModel, PopupAction } from '../lib/types';
  let { model, available = 700 }: {model: ResultModel; available?: number} = $props();
  let buttons = $derived.by(() => { const a = actionTitles(); return [
    {label:a.copy,icon:'copy' as const,action:{type:'copy'} as PopupAction},
    ...(model.showReplace ? [{label:a.replace,icon:'replace' as const,action:{type:'replace'} as PopupAction}] : []),
    {label:a.rewrite,icon:'rewrite' as const,action:{type:'rewrite'} as PopupAction},
    ...(model.showExplain ? [{label:a.explain,icon:'explain' as const,action:{type:'explain'} as PopupAction}] : []),
  ]; });
</script>
<header>
  <!-- Static hand-drawn SVG keyed by the frozen provider value; never model text. -->
  <span class="provider-icon" style:color={providerColors[model.provider]}>{@html providerIcons[model.provider]}</span>
  <span class="provider" title={model.model}>{model.providerName}</span>
  {#if model.operation === 'translate'}
    <span aria-hidden="true">→</span>
    <span class="language">
      <select aria-label={t('Target language')} value={model.targetLanguage} onchange={event => sendAction({type:'retranslate',language:event.currentTarget.value})}>
        {#each model.quickLanguages as language}<option value={language}>{language}</option>{/each}
      </select>
      <span class="chevron" aria-hidden="true">{@html icons.chevron}</span>
    </span>
  {/if}
  {#if model.viaFallback}<span class="fallback">{t('fallback')}</span>{/if}
</header>
{#if model.fallback}
  <details><summary>{t('Why fallback was used')}</summary><p>{t(model.fallback.message.key,model.fallback.message.args)}</p>
    {#each model.fallback.details as line}<p>{t(line.key,line.args)}</p>{/each}
  </details>
{/if}
<div class="text">{model.text}</div>
{#if model.truncated}<p class="warning">{t('Response was truncated')}</p>{/if}
<ActionRow row={model.actionRow} {buttons} retryProviders={model.retryProviders} {available} />
<style>
  /* Mac PopupView header: provider glyph in its colour, then name, arrow and language in ONE font
     (header size, semibold, secondary colour); the language menu is borderless text. */
  header { font-size:var(--header-font-size, 11px); font-weight:600; color:#6b7078; display:flex; align-items:center; gap:4px; flex-wrap:wrap; }
  .provider-icon { display:inline-flex; }
  .provider-icon :global(svg) { width:1.15em; height:1.15em; }
  .language { position:relative; display:inline-flex; align-items:center; max-width:100%; }
  select { appearance:none; -webkit-appearance:none; border:0; background:transparent; padding:0 1em 0 0; margin:0; font:inherit; font-weight:inherit; color:inherit; max-width:100%; cursor:pointer; }
  .chevron { position:absolute; right:0; pointer-events:none; display:inline-flex; }
  .chevron :global(svg) { width:0.85em; height:0.85em; }
  .text { font-size:var(--result-font-size, 13px); }
  .fallback { font-size:11px; color:#a64b08; background:#fff0dc; border-radius:9px; padding:2px 7px; }
</style>
