<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n';
  import { renderError } from '../lib/ipcError';
  import { getPermissions, openPermissionSettings, refreshPermissions, requestPermission } from '../lib/permissionsIpc';
  import { relaunch } from '../lib/settingsIpc';
  import type { PermissionKindId, PermissionStateId, PermissionStatus } from '../lib/settingsTypes';

  let statuses = $state<PermissionStatus[]>([]);
  let failure = $state<string | null>(null);
  let disposed = false;
  // Only the newest read or request is applied (an older response must not undo a newer state).
  let generation = 0;

  async function apply(read: () => Promise<PermissionStatus[]>) {
    const mine = ++generation;
    try {
      const next = await read();
      if (disposed || mine !== generation) return;
      statuses = next;
      failure = null;
    } catch (error) {
      if (disposed || mine !== generation) return;
      failure = typeof error === 'string' ? error : "Couldn't update permissions.";
    }
  }

  onMount(() => {
    // Mount and focus only READ the state; the system prompt is behind the Request buttons.
    void apply(getPermissions);
    // Back from System Settings: read again (the grant takes effect without any action here).
    const onFocus = () => { void apply(refreshPermissions); };
    window.addEventListener('focus', onFocus);
    return () => { disposed = true; window.removeEventListener('focus', onFocus); };
  });

  const request = (kind: PermissionKindId) => apply(() => requestPermission(kind));
  async function open(kind: PermissionKindId) {
    try { await openPermissionSettings(kind); failure = null; }
    catch (error) { failure = typeof error === 'string' ? error : "Couldn't open System Settings."; }
  }
  async function onRelaunch() {
    try { await relaunch(); failure = null; }
    catch (error) { failure = typeof error === 'string' ? error : 'Could not relaunch LingoPop'; }
  }

  function name(kind: PermissionKindId): string {
    return kind === 'accessibility' ? t('Accessibility') : t('Screen Recording');
  }
  function description(kind: PermissionKindId): string {
    return kind === 'accessibility' ? t('Required to read selected text and paste results.') : t('Required for Screen Translate.');
  }
  function stateText(state: PermissionStateId): string {
    switch (state) {
      case 'granted': return t('Granted');
      case 'notDetermined': return t('Not requested yet');
      case 'denied': return t('Denied');
      default: return t('Restricted by a profile or administrator');
    }
  }
</script>
<section aria-labelledby="permissions-title">
  <h3 id="permissions-title">{t('Permissions')}</h3>
  {#each statuses as status (status.kind)}
    <div class="permission" data-permission={status.kind}>
      <div class="text">
        <span class="name">{name(status.kind)}</span>
        <span class="description">{description(status.kind)}</span>
      </div>
      <span class="state" class:granted={status.state === 'granted'}>{stateText(status.state)}</span>
      <span class="actions">
        {#if status.state === 'notDetermined' || status.state === 'denied'}
          <button type="button" onclick={() => request(status.kind)}>{t('Request')}</button>
        {/if}
        {#if status.state !== 'granted'}
          <button type="button" onclick={() => open(status.kind)}>{t('Open Settings')}</button>
        {/if}
      </span>
    </div>
    {#if status.relaunchHint}
      <div class="relaunch">
        <span role="status">{t('After granting access, fully quit and relaunch LingoPop.')}</span>
        <button type="button" onclick={onRelaunch}>{t('Relaunch')}</button>
      </div>
    {/if}
  {/each}
  <div class="footer">
    <button type="button" onclick={() => apply(refreshPermissions)}>{t('Re-check')}</button>
  </div>
  {#if failure}<p class="error" role="alert">{renderError(failure)}</p>{/if}
</section>
<style>
  section { padding:10px 12px; background:#fff; border:1px solid #e1e5ea; border-radius:8px; display:flex; flex-direction:column; gap:8px; }
  h3 { margin:0; font-size:11px; font-weight:600; text-transform:uppercase; letter-spacing:.04em; color:#596574; }
  .permission { display:flex; align-items:center; gap:12px; }
  .text { display:flex; flex-direction:column; flex:1; min-width:0; }
  .description { font-size:12px; color:#596574; }
  .state { font-size:12px; color:#a02c12; white-space:nowrap; }
  .state.granted { color:#1b7a3d; }
  .actions { display:flex; gap:6px; }
  .relaunch { display:flex; align-items:center; justify-content:space-between; gap:8px; font-size:12px; color:#596574; }
  .footer { display:flex; justify-content:flex-end; }
  .error { margin:0; font-size:12px; color:#a02c12; }
</style>
