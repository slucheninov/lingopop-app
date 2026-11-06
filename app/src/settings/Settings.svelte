<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { t } from '../lib/i18n';
  import { renderError } from '../lib/ipcError';
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
  // One serialized queue for every read and write of the view. Only the result of the LAST queued job is
  // applied, so the display always ends on a read or response taken after the last commit. A
  // `settings://changed` event is only a SIGNAL (events and command responses travel on different channels
  // with no ordering guarantee, so its payload may be older than what is displayed): it queues one fresh read.
  let pending = 0;
  let readQueued = false;
  let queue: Promise<unknown> = Promise.resolve();
  /** Resolves `true` when the call succeeded (a write: the shell saved the change). */
  function enqueue(send: () => Promise<SettingsView>, kind: 'write' | 'read', reportFailure: boolean, onError?: (key: string) => void): Promise<boolean> {
    pending++;
    const job = queue.then(async () => {
      if (kind === 'read') readQueued = false;
      let next: SettingsView | null = null;
      let rejection: unknown;
      let ok = true;
      try { next = await send(); } catch (error) { ok = false; rejection = error; }
      pending--;
      if (disposed) return ok;
      if (ok) {
        if (pending === 0) view = next;
        if (reportFailure) failure = null;
      } else {
        if (reportFailure) {
          if (onError) onError(typeof rejection === 'string' ? rejection : "Couldn't save settings.");
          else show(rejection);
        }
        if (kind === 'write' && pending === 0) {
          // The refused change is not persisted: show what is (outside the queue, which this job still holds).
          try { const saved = await getSettings(); if (!disposed && pending === 0) view = saved; } catch { /* keep the error */ }
        }
      }
      return ok;
    });
    queue = job;
    return job;
  }
  const load = () => enqueue(getSettings, 'read', true);
  const signal = () => { if (!readQueued) { readQueued = true; void enqueue(getSettings, 'read', false); } };
  const mutate = (send: () => Promise<SettingsView>) => enqueue(send, 'write', true);
  const onPatch = (patch: SettingsPatch, onError?: (key: string) => void) => enqueue(() => updateSetting(patch), 'write', true, onError);
  const onUiPatch = (patch: UiPatch) => mutate(() => updateUi(patch));

  onMount(() => {
    const changed = listen<unknown>('settings://changed', () => { if (!disposed) signal(); });
    // A change committed between the first read and the registration (the startup update check)
    // emitted its signal to nobody: read once more as soon as the listener is in place.
    void changed.then(() => { if (!disposed) signal(); }).catch(() => {});
    const registrations = [
      changed,
      listen<string>('settings://tab', event => { if (!disposed && isTab(event.payload)) active = event.payload; }),
    ];
    void load();
    return () => {
      disposed = true;
      for (const registration of registrations) void registration.then(unlisten => unlisten()).catch(() => {});
    };
  });
</script>
<div class="frame">
  <Tabs {active} onSelect={tab => { active = tab; }} />
  <div class="panel" role="tabpanel" id={`panel-${active}`} aria-labelledby={`tab-${active}`} aria-label={TITLES[active]()}>
    {#if failure}<p class="error" role="alert">{renderError(failure)}</p>{/if}
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
