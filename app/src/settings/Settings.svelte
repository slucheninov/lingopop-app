<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { t } from '../lib/i18n';
  import { getSettings, updateSetting, updateUi } from '../lib/settingsIpc';
  import type { SettingsPatch, SettingsView, TabId, UiPatch } from '../lib/settingsTypes';
  import Tabs from './Tabs.svelte';
  // The tab bodies load lazily: only the opened tab's chunk is fetched.
  const BODIES: Record<TabId, () => Promise<{ default: typeof import('./MainTab.svelte').default }>> = {
    main: () => import('./MainTab.svelte'),
    providers: () => import('./ProvidersTab.svelte') as never,
    shortcuts: () => import('./ShortcutsTab.svelte') as never,
    usage: () => import('./UsageTab.svelte') as never,
    magic: () => import('./MagicTab.svelte') as never,
    about: () => import('./AboutTab.svelte') as never,
  };
  const TITLES: Record<TabId, () => string> = {
    main: () => t('Main'), providers: () => t('Providers'), shortcuts: () => t('Shortcuts'),
    usage: () => t('Usage'), magic: () => t('Magic'), about: () => t('About'),
  };
  const isTab = (value: unknown): value is TabId => typeof value === 'string' && Object.hasOwn(BODIES, value);

  let { initialTab = null }: { initialTab?: TabId | null } = $props();
  let active = $state<TabId>('main');
  let view = $state<SettingsView | null>(null);
  let failure = $state<string | null>(null);
  let disposed = false;
  $effect.pre(() => { if (isTab(initialTab)) active = initialTab; });

  /** A rejection is a catalog key from the shell; anything else shows the generic save message. */
  const show = (error: unknown) => { failure = typeof error === 'string' ? error : "Couldn't save settings."; };
  async function run(request: Promise<SettingsView>) {
    try {
      const next = await request;
      if (!disposed) { view = next; failure = null; }
    } catch (error) {
      if (!disposed) show(error);
    }
  }
  const onPatch = (patch: SettingsPatch) => run(updateSetting(patch));
  const onUiPatch = (patch: UiPatch) => run(updateUi(patch));

  onMount(() => {
    const registrations = [
      listen<SettingsView>('settings://changed', event => { if (!disposed) view = event.payload; }),
      listen<string>('settings://tab', event => { if (!disposed && isTab(event.payload)) active = event.payload; }),
    ];
    void run(getSettings());
    return () => {
      disposed = true;
      for (const registration of registrations) void registration.then(unlisten => unlisten()).catch(() => {});
    };
  });
</script>
<div class="frame">
  <Tabs {active} onSelect={tab => { active = tab; }} />
  <div class="panel" role="tabpanel" id={`panel-${active}`} aria-labelledby={`tab-${active}`} aria-label={TITLES[active]()}>
    {#if failure}<p class="error" role="alert">{t(failure)}</p>{/if}
    {#if view}
      {#key active}
        {#await BODIES[active]() then body}
          {@const Body = body.default}
          <Body {view} {onPatch} {onUiPatch} />
        {/await}
      {/key}
    {/if}
  </div>
</div>
<style>
  :global(html), :global(body) { height:100%; overflow:hidden; }
  :global(body) { margin:0; font:13px system-ui,sans-serif; color:#25282c; background:#f3f5f8; }
  :global(*) { box-sizing:border-box; }
  :global(button), :global(select) { font:inherit; color:inherit; background:#fff; border:1px solid #cbd0d7; border-radius:5px; padding:5px 8px; }
  :global(button) { display:inline-flex; align-items:center; justify-content:center; gap:6px; cursor:pointer; }
  :global(button:hover) { background:#edf2f8; }
  :global(button:focus-visible), :global(select:focus-visible), :global(input:focus-visible) { outline:2px solid #3478cf; outline-offset:2px; }
  .frame { display:flex; flex-direction:column; height:100vh; }
  .panel { flex:1; overflow:auto; padding:14px 16px; }
  .error { margin:0 0 10px; padding:6px 10px; border-radius:6px; font-size:12px; color:#7a2a12; background:#fdeee8; }
</style>
