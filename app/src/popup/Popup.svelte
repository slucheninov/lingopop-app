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
    let disposed = false;
    const registration = listen<Record<string, unknown>>('popup://model', (event) => {
      model = event.payload;
    });
    // Only tell the shell we are ready once the listener is registered, or the re-emitted
    // model could be lost.
    void registration.then(() => {
      if (!disposed) void invoke('popup_ready');
    });
    return () => {
      disposed = true;
      void registration.then((unlisten) => unlisten());
    };
  });
</script>

<main>
  {#if model}
    <p>{String(model.phase)}</p>
    <p>{String(model.text ?? model.message ?? '')}</p>
  {/if}
</main>
