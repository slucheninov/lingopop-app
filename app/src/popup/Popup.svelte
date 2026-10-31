<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { ready, resized, sendAction } from '../lib/ipc';
  import { popupWidth } from '../lib/actionRow';
  import { icons } from '../lib/icons';
  import type { PopupModel } from '../lib/types';
  import LoadingView from './LoadingView.svelte';
  import ResultView from './ResultView.svelte';
  import OcrView from './OcrView.svelte';
  import ErrorView from './ErrorView.svelte';
  let model = $state<PopupModel | null>(null);
  let content = $state<HTMLElement>();
  let disposed = false;
  async function report() {
    await tick();
    if (disposed || !model || !content) return;
    const rect = content.getBoundingClientRect();
    await resized(Math.ceil(rect.width), Math.ceil(rect.height)).catch(() => {});
  }
  $effect(() => { void model; void report(); });
  onMount(() => {
    const observer = new ResizeObserver(() => { void report(); });
    if (content) observer.observe(content);
    const registration = listen<PopupModel>('popup://model', event => { if (!disposed) model = event.payload; });
    // Register first: popup_ready re-emits a model that may have arrived before mount.
    void registration.then(() => { if (!disposed) void ready().catch(() => {}); }).catch(() => {});
    return () => {
      disposed = true;
      observer.disconnect();
      void registration.then(unlisten => unlisten()).catch(() => {});
    };
  });
</script>
<svelte:window onkeydown={event => { if (event.key === 'Escape') sendAction({type:'close'}); }} />
<main bind:this={content} style:width={`${model ? popupWidth(model) : 320}px`}>
  {#if model}
    <button class="close" type="button" title="Close" aria-label="Close" onclick={() => sendAction({type:'close'})}>{@html icons.close}</button>
    {#key model}
      {#if model.phase === 'loading'}<LoadingView {model} />
      {:else if model.phase === 'result'}<ResultView {model} />
      {:else if model.phase === 'ocr'}<OcrView {model} />
      {:else}<ErrorView {model} />{/if}
    {/key}
  {/if}
</main>
<style>
  :global(*) { box-sizing:border-box; }
  :global(body) { margin:0; font:13px system-ui,sans-serif; color:#25282c; background:transparent; }
  main { position:relative; padding:14px; background:#fafbfc; border:1px solid #d5d9df; border-radius:10px; }
  .close { position:absolute; top:7px; right:7px; padding:3px; border:0; background:transparent; }
  :global(button), :global(select) { font:inherit; color:inherit; background:#fff; border:1px solid #cbd0d7; border-radius:5px; padding:5px 8px; }
  :global(button) { display:inline-flex; align-items:center; justify-content:center; gap:6px; cursor:pointer; flex-shrink:0; }
  :global(button:hover) { background:#edf2f8; }
  :global(button:disabled) { opacity:.5; cursor:default; }
  :global(button:focus-visible), :global(select:focus-visible), :global(summary:focus-visible) { outline:2px solid #3478cf; outline-offset:2px; }
  :global(.text) { white-space:pre-wrap; overflow-wrap:anywhere; user-select:text; line-height:1.5; max-height:420px; overflow:auto; margin-top:12px; }
  :global(p), :global(details) { overflow-wrap:anywhere; }
  :global(details) { margin-top:10px; }
  :global(summary) { cursor:pointer; color:#596574; }
  :global(.warning) { color:#a64b08; }
</style>
