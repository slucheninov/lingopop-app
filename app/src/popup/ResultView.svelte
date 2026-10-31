<script lang="ts">
  import ActionRow from './ActionRow.svelte';
  import { t } from '../lib/i18n';
  import { actionTitles } from '../lib/actionRow';
  import { sendAction } from '../lib/ipc';
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
  <strong title={model.model}>{model.providerName}</strong>
  {#if model.operation === 'translate'}
    <span aria-hidden="true">→</span>
    <select aria-label={t('Target language')} value={model.targetLanguage} onchange={event => sendAction({type:'retranslate',language:event.currentTarget.value})}>
      {#each model.quickLanguages as language}<option value={language}>{language}</option>{/each}
    </select>
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
  header { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }
  select { max-width:100%; }
  .fallback { font-size:11px; color:#a64b08; background:#fff0dc; border-radius:9px; padding:2px 7px; }
</style>
