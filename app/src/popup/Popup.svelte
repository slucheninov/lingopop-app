<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';

  // Skeleton: Task 8 replaces this with the real popup UI.
  let model = $state<Record<string, unknown> | null>(null);

  async function report() {
    await tick();
    const rect = document.body.getBoundingClientRect();
    await invoke('popup_resized', { width: rect.width, height: rect.height });
  }

  $effect(() => {
    // Re-measure after every model change.
    void model;
    void report();
  });

  onMount(() => {
    const unlisten = listen<Record<string, unknown>>('popup://model', (event) => {
      model = event.payload;
    });
    void invoke('popup_ready');
    return () => {
      void unlisten.then((fn) => fn());
    };
  });
</script>

<main>
  {#if model}
    <p>{String(model.phase)}</p>
    <p>{String(model.text ?? model.message ?? '')}</p>
  {/if}
</main>
