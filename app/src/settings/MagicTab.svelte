<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '../lib/i18n';
  import { countScalars, dropControlsAndTruncate, sanitizeMagicPrompt } from '../lib/scalars';
  import type { SettingsPatch, SettingsView, UiPatch } from '../lib/settingsTypes';

  // Port of MagicSettingsTab.swift. The frame passes the Ui patch callback to every tab; Magic has no UI preference.
  let { view, onPatch }: { view: SettingsView; onPatch: (patch: SettingsPatch) => Promise<boolean>; onUiPatch?: (patch: UiPatch) => Promise<boolean> } = $props();

  const LIMIT = 2000;
  const WARN_AT = 1900;

  // `saved` is the last committed prompt we know of (a saved value, or what the view last reported); the draft starts
  // from it (Mac: onAppear) and follows it while the user has no unsaved edit.
  let saved = $state(untrack(() => view.magicPrompt));
  let draft = $state(untrack(() => view.magicPrompt));
  let confirmReset = $state(false);
  let cancelButton = $state<HTMLButtonElement | null>(null);
  let count = $derived(countScalars(draft));

  $effect(() => {
    if (confirmReset) cancelButton?.focus();
  });
  // A view is authoritative whenever it changes (a signal read, a later response): an unedited draft follows it.
  $effect(() => {
    const next = view.magicPrompt;
    untrack(() => {
      if (draft === saved) draft = next;
      saved = next;
    });
  });

  function onInput(event: Event) {
    const area = event.currentTarget as HTMLTextAreaElement;
    // Mac cuts to 2000 scalars live and sanitizes on Save; controls are dropped first so none can use up the limit.
    const next = dropControlsAndTruncate(area.value, LIMIT);
    draft = next;
    // The draft may equal its previous value (typing past the limit), so write the cut text back to the field.
    if (area.value !== next) area.value = next;
  }

  /**
   * A successful save commits `sanitizeMagicPrompt(value)` (the shell applies the same rule). The draft adopts it unless the
   * user kept typing meanwhile; the answer is not read from `view`, which may still hold an older commit when a queued read
   * superseded this response. A refused save keeps the draft (the frame shows the error).
   */
  async function save(value: string) {
    if (!(await onPatch({ field: 'magicPrompt', value }))) return;
    const committed = sanitizeMagicPrompt(value);
    if (draft === value) draft = committed;
    saved = committed;
  }

  function reset() {
    confirmReset = false;
    draft = view.defaultMagicPrompt;
    void save(draft);
  }
</script>
<section class="magic">
  <p class="hint">{t('Magic runs a single AI call with your custom system prompt. Use {{targetLanguage}} to insert your configured target language automatically.')}</p>
  <label for="magic-prompt" class="caption">{t('System prompt')}</label>
  <textarea id="magic-prompt" value={draft} oninput={onInput} spellcheck="false" rows="8"></textarea>
  <div class="row">
    <span class="count" class:warn={count >= WARN_AT}>{t('%lld / 2000', [count])}</span>
    <span class="spacer"></span>
    <button type="button" class="link" onclick={() => { confirmReset = true; }}>{t('Reset to default')}</button>
    <button type="button" class="primary" disabled={draft === saved} onclick={() => { void save(draft); }}>{t('Save')}</button>
  </div>
  {#if confirmReset}
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="magic-reset-title">
      <p id="magic-reset-title" class="title">{t('Reset prompt?')}</p>
      <p class="message">{t('This will replace your prompt with the default. Continue?')}</p>
      <div class="actions">
        <button type="button" bind:this={cancelButton} onclick={() => { confirmReset = false; }}>{t('Cancel')}</button>
        <button type="button" class="danger" onclick={reset}>{t('Reset')}</button>
      </div>
    </div>
  {/if}
</section>
<style>
  .magic { display: flex; flex-direction: column; gap: 8px; }
  .hint { margin: 0; font-size: 12px; color: #596574; }
  .caption { font-size: 11px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; color: #596574; }
  textarea { width: 100%; min-height: 120px; resize: vertical; padding: 8px; font: 13px ui-monospace, Consolas, monospace; color: inherit; background: #fff; border: 1px solid #cbd0d7; border-radius: 6px; }
  textarea:focus-visible { outline: 2px solid #3478cf; outline-offset: 2px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .count { font-size: 11px; color: #596574; font-variant-numeric: tabular-nums; }
  .count.warn { color: #ff3b30; }
  .link { border: 0; background: none; color: #3478cf; }
  .primary { color: #fff; background: #3478cf; border-color: #3478cf; }
  .primary:hover { background: #2b66b3; }
  .primary:disabled { opacity: 0.45; cursor: default; background: #3478cf; }
  .dialog { display: flex; flex-direction: column; gap: 8px; padding: 12px; background: #fff; border: 1px solid #e1e5ea; border-radius: 8px; }
  .dialog p { margin: 0; }
  .title { font-weight: 600; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .danger { color: #fff; background: #ff3b30; border-color: #ff3b30; }
</style>
