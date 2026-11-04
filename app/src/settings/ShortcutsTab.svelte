<script lang="ts">
  import type { SettingsPatch, SettingsView, UiPatch } from '../lib/settingsTypes';
  import type { Operation } from '../lib/types';
  import { renderError } from '../lib/ipcError';
  import { currentHost } from '../lib/bootstrap';
  import { t } from '../lib/i18n';
  import HotkeyRecorder from './HotkeyRecorder.svelte';
  let { view, onPatch }: { view: SettingsView; onPatch: (patch: SettingsPatch, onError?: (key: string) => void) => Promise<boolean>; onUiPatch?: (patch: UiPatch) => Promise<boolean> } = $props();
  let errors = $state<Partial<Record<Operation, string>>>({});
  async function patch(operation: Operation, hotkey: import('../lib/settingsTypes').Hotkey | null) {
    delete errors[operation];
    const accepted = await onPatch({ field: 'hotkey', value: { operation, hotkey } }, key => { errors[operation] = key; });
    if (accepted) delete errors[operation];
    return accepted;
  }
  const rows: { operation: Operation; name: () => string; icon: string }[] = [
    { operation: 'translate', name: () => t('Translate'), icon: '文' },
    { operation: 'grammarFix', name: () => t('Fix grammar'), icon: '✓' },
    { operation: 'rewrite', name: () => t('Rewrite'), icon: '✎' },
    { operation: 'explain', name: () => t('Explain'), icon: '?' },
    { operation: 'screenTranslate', name: () => t('Screen Translate'), icon: '▣' },
    { operation: 'magic', name: () => t('Magic'), icon: '✦' },
  ];
</script>
<section>
  {#if view.platform === 'macos' && view.swiftRunning}
    <p role="status">{t('Quit the Swift LingoPop to enable shortcuts in LingoPop Preview.')}</p>
  {/if}
  {#each rows as row}
    <div class="row">
      <span class="label"><span aria-hidden="true">{row.icon}</span> {row.name()}</span>
      <div class="field">
        <div class="controls">
          <HotkeyRecorder label={row.name()} hotkey={view.hotkeys[row.operation]} onRecord={hotkey => patch(row.operation, hotkey)} />
          {#if view.hotkeys[row.operation]}
            <button aria-label={t('Clear %@', [row.name()])} onclick={() => { void patch(row.operation, null); }}>✕</button>
          {/if}
        </div>
        {#if errors[row.operation]}
          <p class="error" role="alert">{renderError(errors[row.operation]!)}</p>
        {:else if view.hotkeyStatus[row.operation] === 'swiftsuspended'}
          <p>{t('Suspended while Swift LingoPop is running.')}</p>
        {:else if view.hotkeyStatus[row.operation] === 'conflict'}
          <p class="error">{t('This shortcut is already used by another app.')}</p>
        {:else if view.hotkeyStatus[row.operation] === 'invalid'}
          <p class="error">{t("This shortcut can't be registered.")}</p>
        {/if}
      </div>
    </div>
  {/each}
  <p class="footnote">{currentHost() === 'unix' ? t('Click a shortcut field, then press a key combination. Use ⌘, Option, Control or Shift plus a key. Esc cancels recording. ✕ removes the shortcut.') : t('Click a shortcut field, then press a key combination. Use Ctrl, Alt or Shift plus a key. Esc cancels recording. ✕ removes the shortcut.')}</p>
</section>
<style>
  section { background:white; border:1px solid #dce0e6; border-radius:8px; padding:4px 14px; }
  .row { display:flex; justify-content:space-between; gap:16px; padding:10px 0; border-bottom:1px solid #e8ebef; }
  .label { padding-top:3px; }
  .label span { display:inline-block; width:22px; color:#647080; }
  .field { width:330px; }
  .controls { display:flex; gap:8px; align-items:center; }
  .controls > button { padding:0; border:0; background:none; color:#647080; }
  .error { color:#b32323; font-size:12px; margin:5px 0 0; }
  .footnote { color:#647080; font-size:12px; line-height:1.5; }
</style>
