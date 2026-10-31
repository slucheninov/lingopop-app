<script lang="ts">
  import ActionRow from './ActionRow.svelte';
  import { popupWidth } from '../lib/actionRow';
  import type { OcrModel, PopupAction } from '../lib/types';
  let {model}: {model: OcrModel} = $props();
  let provider = $derived(model.translateProviders.find(p => p.provider === model.activeTranslateProvider) ?? model.translateProviders[0]);
  let buttons = $derived([
    {label:'Copy',icon:'copy' as const,action:{type:'copy'} as PopupAction},
    {label:'Translate',icon:'translate' as const,action:{type:'translateOcr',provider:provider?.provider ?? 'Claude'} as PopupAction,disabled:!provider},
  ]);
</script>
<header><strong>Recognized text</strong></header>
<div class="text">{model.text}</div>
<ActionRow row={model.actionRow} {buttons} retryProviders={model.retryProviders} translateProviders={model.translateProviders} available={popupWidth(model)} />
