<script lang="ts">
  import { onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { classifyKeydown } from '../lib/hotkeyRecorder';
  import { currentHost } from '../lib/bootstrap';
  import { t } from '../lib/i18n';
  import { renderError } from '../lib/ipcError';
  import type { Hotkey, HotkeyView } from '../lib/settingsTypes';
  let { label, hotkey, onRecord }: { label: string; hotkey?: HotkeyView; onRecord: (hotkey: Hotkey) => Promise<boolean> } = $props();
  const host = currentHost();
  const prompt = () => host === 'unix' ? t('Press a key combo (⌘ / Option)…') : t('Press a key combo…');
  let recording = $state(false);
  let busy = $state(false);
  let failure = $state<string | null>(null);
  let disposed = false;
  let button: HTMLButtonElement | undefined = $state();
  // Suspend calls are chained: each waits for the previous one to settle, so a `false` (blur, Esc, unmount)
  // can never reach the shell before the `true` it undoes, whatever order the async Tauri tasks would run in.
  let inFlight: Promise<unknown> = Promise.resolve();
  function suspend(suspended: boolean): Promise<unknown> {
    const run = inFlight.then(() => invoke('hotkeys_suspend', { suspended }));
    inFlight = run.catch(() => {});
    return run;
  }
  async function stop() {
    if (!recording) return;
    recording = false;
    try { await suspend(false); }
    catch (e) { if (!disposed) failure = typeof e === 'string' ? e : "Couldn't save settings."; }
  }
  async function toggle() {
    if (busy) return;
    if (recording) { await stop(); return; }
    recording = true;
    // WebKit (macOS WKWebView) does not focus a <button> on click, so its keydown would never
    // fire; focus it explicitly so the recorder receives the chord on every engine.
    button?.focus();
    failure = null;
    busy = true;
    try { await suspend(true); }
    catch (e) { recording = false; failure = typeof e === 'string' ? e : "Couldn't save settings."; }
    finally { busy = false; }
  }
  async function keydown(event: KeyboardEvent) {
    if (!recording || busy) return;
    event.preventDefault(); event.stopPropagation();
    const result = classifyKeydown(event);
    if (result.kind === 'ignored') return;
    if (result.kind === 'cancel') { await stop(); return; }
    busy = true;
    try { await onRecord({ key: result.key, modifiers: result.modifiers }); }
    finally { busy = false; await stop(); }
  }
  onDestroy(() => { disposed = true; void stop(); });
</script>
<button bind:this={button} class:recording aria-label={label} aria-pressed={recording} title={recording ? prompt() : hotkey?.display ?? ''}
  onclick={() => { void toggle(); }} onkeydown={keydown} onblur={() => { void stop(); }}>
  {recording ? prompt() : hotkey?.display ?? '—'}
</button>
{#if failure}<p role="alert">{renderError(failure)}</p>{/if}
<style>
  button { width:160px; height:24px; padding:0 6px; font-size:12px; }
  .recording { outline:2px solid #3478cf; }
  p { color:#b32323; font-size:12px; }
</style>
