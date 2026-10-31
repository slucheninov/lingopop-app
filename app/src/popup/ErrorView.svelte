<script lang="ts">
  import ActionRow from './ActionRow.svelte';
  import { t } from '../lib/i18n';
  import type { ErrorModel } from '../lib/types';
  let {model, available = 318}: {model:ErrorModel; available?: number} = $props();
</script>
<header><strong>Error</strong></header>
<p role="alert">{t(model.message.key,model.message.args)}</p>
{#if model.details.length}<details><summary>Details</summary>{#each model.details as line}<p>{t(line.key,line.args)}</p>{/each}</details>{/if}
<ActionRow row={{preferred:'full',candidates:['full','compact']}} buttons={[]} retryProviders={model.retryProviders} {available} />
