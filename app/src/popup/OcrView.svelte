<script lang="ts">
  import ActionRow from './ActionRow.svelte';
  import { t } from '../lib/i18n';
  import { actionTitles } from '../lib/actionRow';
  import type { OcrModel, PopupAction } from '../lib/types';
  let {model, available = 700}: {model: OcrModel; available?: number} = $props();
  let provider = $derived(model.translateProviders.find(p => p.provider === model.activeTranslateProvider) ?? model.translateProviders[0]);
  let buttons = $derived.by(() => { const a = actionTitles(); return [
    {label:a.copy,icon:'copy' as const,action:{type:'copy'} as PopupAction},
    {label:a.translate,icon:'translate' as const,action:{type:'translateOcr',provider:provider?.provider ?? 'Claude'} as PopupAction,disabled:!provider},
  ]; });
</script>
<header>{t('Recognized text')}</header>
<div class="text">{model.text}</div>
<ActionRow row={model.actionRow} {buttons} retryProviders={model.retryProviders} translateProviders={model.translateProviders} {available} />
<style>
  header { font-size:var(--header-font-size, 11px); font-weight:600; color:#6b7078; }
  .text { font-size:var(--result-font-size, 13px); }
</style>
