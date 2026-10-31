<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { ready, resized, sendAction } from '../lib/ipc';
  import { panelWidth, rowAvailable, viewKey } from '../lib/actionRow';
  import { createResizeReporter } from '../lib/resizeReporter';
  import { icons } from '../lib/icons';
  import { t } from '../lib/i18n';
  import type { PopupModel } from '../lib/types';
  import LoadingView from './LoadingView.svelte';
  import ResultView from './ResultView.svelte';
  import OcrView from './OcrView.svelte';
  import ErrorView from './ErrorView.svelte';
  let model = $state<PopupModel | null>(null);
  let content = $state<HTMLElement>();
  let disposed = false;
  const reporter = createResizeReporter(resized);
  let frame = 0;
  // Coalesce model updates and observer callbacks into one measurement per frame.
  function report() {
    if (frame || disposed) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      if (disposed || !model || !content) return;
      const rect = content.getBoundingClientRect();
      void reporter.report(Math.ceil(rect.width), Math.ceil(rect.height));
    });
  }
  let width = $derived(model ? panelWidth(model) : 320);
  $effect(() => { void model; report(); });
  onMount(() => {
    const observer = new ResizeObserver(() => report());
    if (content) observer.observe(content);
    const registration = listen<PopupModel>('popup://model', event => { if (!disposed) model = event.payload; });
    // Register first: popup_ready re-emits a model that may have arrived before mount.
    void registration.then(() => { if (!disposed) void ready().catch(() => {}); }).catch(() => {});
    return () => {
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      observer.disconnect();
      void registration.then(unlisten => unlisten()).catch(() => {});
    };
  });
</script>
<svelte:window onkeydown={event => { if (event.key === 'Escape') sendAction({type:'close'}); }} />
<main bind:this={content} style:width={`${width}px`}>
  {#if model}
    <button class="close" type="button" title={t('Close')} aria-label={t('Close')} onclick={() => sendAction({type:'close'})}>{@html icons.close}</button>
    {#key viewKey(model)}
      {#if model.phase === 'loading'}<LoadingView {model} />
      {:else if model.phase === 'result'}<ResultView {model} available={rowAvailable(width)} />
      {:else if model.phase === 'ocr'}<OcrView {model} available={rowAvailable(width)} />
      {:else}<ErrorView {model} available={rowAvailable(width)} />{/if}
    {/key}
    {#if model.notice}<p class="notice" role="status">{t(model.notice.key, model.notice.args)}</p>{/if}
  {/if}
</main>
<style>
  :global(*) { box-sizing:border-box; }
  :global(html), :global(body) { overflow:hidden; }
  :global(body) { margin:0; font:13px system-ui,sans-serif; color:#25282c; background:transparent; }
  main { position:relative; padding:14px; background:#fafbfc; border:1px solid #d5d9df; border-radius:10px; }
  .notice { margin:8px 0 0; padding:5px 8px; border-radius:6px; font-size:12px; color:#7a2a12; background:#fdeee8; }
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
  :global(header) { padding-right:22px; }
  :global(.warning) { color:#a64b08; }
</style>
