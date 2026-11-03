<script lang="ts">
  import type { SettingsPatch, SettingsView, UiPatch } from '../lib/settingsTypes';
  import type { Provider } from '../lib/types';
  import ProviderForm from './ProviderForm.svelte';
  import ProviderSidebar from './ProviderSidebar.svelte';
  let { view, onPatch }: { view: SettingsView; onPatch: (patch: SettingsPatch) => Promise<boolean>; onUiPatch?: (patch: UiPatch) => Promise<boolean> } = $props();
  // Until the user picks one, the active provider is selected (mac `onAppear { selected = activeProvider }`).
  let chosen = $state<Provider | null>(null);
  const selected = $derived(chosen ?? view.activeProvider);
</script>
<div class="providers">
  <ProviderSidebar providers={view.providers} active={view.activeProvider} {selected} onSelect={provider => { chosen = provider; }} />
  <div class="detail">
    <!-- Keyed: switching provider recreates the form, which discards its draft and the key field. -->
    {#key selected}<ProviderForm provider={selected} {view} {onPatch} />{/key}
  </div>
</div>
<style>
  .providers { display:flex; align-items:stretch; margin:-14px -16px; min-height:calc(100% + 28px); }
  .detail { flex:1; min-width:0; padding:14px 16px; overflow-y:auto; }
</style>
