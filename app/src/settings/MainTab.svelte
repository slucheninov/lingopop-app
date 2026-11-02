<script lang="ts">
  import { t } from '../lib/i18n';
  import type { AppLanguage, SettingsPatch, SettingsView, UiPatch } from '../lib/settingsTypes';
  import LanguagePairs from './LanguagePairs.svelte';
  import Relaunch from './Relaunch.svelte';
  let { view, onPatch, onUiPatch }: { view: SettingsView; onPatch: (patch: SettingsPatch) => void; onUiPatch: (patch: UiPatch) => void } = $props();

  // Native names are never translated; `System` is.
  const APP_LANGUAGES: { value: AppLanguage; name: string | null }[] = [
    { value: 'system', name: null }, { value: 'en', name: 'English' }, { value: 'pl', name: 'Polski' },
    { value: 'uk', name: 'Українська' }, { value: 'de', name: 'Deutsch' },
  ];
  let targetOptions = $derived(view.quickLanguages.includes(view.targetLanguage) ? view.quickLanguages : [view.targetLanguage, ...view.quickLanguages]);

  // The slider shows the dragged value at once and follows the saved one when it changes from outside.
  let fontSize = $state(0);
  let sentFontSize = 0;
  $effect(() => { fontSize = view.popupFontSize; sentFontSize = view.popupFontSize; });
  function onFont(event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(value)) return;
    fontSize = value;
    if (value === sentFontSize) return;
    sentFontSize = value;
    onUiPatch({ field: 'popupFontSize', value });
  }

  /** A whole number in `min..=max`; garbage is ignored and the field shows the saved value again. */
  function onNumber(event: Event, field: 'maxInputChars' | 'maxHistoryCount' | 'menuHistoryCount', max: number) {
    const input = event.currentTarget as HTMLInputElement;
    const parsed = Number(input.value);
    if (input.value.trim() === '' || !Number.isFinite(parsed)) {
      input.value = String(view[field]);
      return;
    }
    const value = Math.min(max, Math.max(0, Math.trunc(parsed)));
    input.value = String(value);
    onPatch({ field, value });
  }
</script>
<div class="main">
  <section>
    <h3>{t('Target language')}</h3>
    <label class="row switch"><input type="checkbox" role="switch" checked={view.useAutoDetect} onchange={event => onPatch({ field: 'useAutoDetect', value: event.currentTarget.checked })} /><span>{t('Auto-detect input language')}</span></label>
    <label class="row"><span>{view.useAutoDetect ? t('Default target') : t('Target')}</span>
      <select value={view.targetLanguage} onchange={event => onPatch({ field: 'targetLanguage', value: event.currentTarget.value })}>
        {#each targetOptions as language}<option value={language}>{language}</option>{/each}
      </select></label>
    {#if view.useAutoDetect}
      <LanguagePairs pairs={view.languagePairs} languages={view.quickLanguages} onChange={value => onPatch({ field: 'languagePairs', value })} />
    {/if}
  </section>
  <section>
    <h3>{t('Popup')}</h3>
    <label class="row"><span>{t('Result font size')}</span>
      <span class="slider"><input type="range" aria-label={t('Result font size')} min="11" max="20" step="1" value={fontSize} oninput={onFont} onchange={onFont} />
        <output>{t('%lld pt', [fontSize])}</output></span></label>
  </section>
  <section>
    <h3>{t('Language')}</h3>
    <label class="row"><span>{t('Language')}</span>
      <select value={view.appLanguage} onchange={event => onUiPatch({ field: 'appLanguage', value: event.currentTarget.value as AppLanguage })}>
        {#each APP_LANGUAGES as language}<option value={language.value}>{language.name ?? t('System')}</option>{/each}
      </select></label>
    <Relaunch needed={view.needsRelaunch} />
  </section>
  <section>
    <h3>{t('Limits')}</h3>
    <label class="row"><span>{t('Max input characters')}</span>
      <input type="number" min="0" step="1" value={view.maxInputChars} onchange={event => onNumber(event, 'maxInputChars', 4294967295)} /></label>
    <label class="row"><span>{t('History size')}</span>
      <input type="number" min="0" max="100" step="1" value={view.maxHistoryCount} onchange={event => onNumber(event, 'maxHistoryCount', 100)} /></label>
    <label class="row"><span>{t('Display')}</span>
      <input type="number" min="0" max="100" step="1" value={view.menuHistoryCount} onchange={event => onNumber(event, 'menuHistoryCount', 100)} /></label>
  </section>
  <section>
    <h3>{t('Startup')}</h3>
    <label class="row switch"><input type="checkbox" role="switch" checked={view.launchAtLogin} onchange={event => onPatch({ field: 'launchAtLogin', value: event.currentTarget.checked })} /><span>{t('Launch at login')}</span></label>
    <label class="row switch"><input type="checkbox" role="switch" checked={view.autoCopyOCR} onchange={event => onPatch({ field: 'autoCopyOCR', value: event.currentTarget.checked })} /><span>{t('Auto-copy OCR/translation results to clipboard')}</span></label>
  </section>
</div>
<style>
  .main { display:flex; flex-direction:column; gap:14px; }
  section { padding:10px 12px; background:#fff; border:1px solid #e1e5ea; border-radius:8px; display:flex; flex-direction:column; gap:8px; }
  h3 { margin:0; font-size:11px; font-weight:600; text-transform:uppercase; letter-spacing:.04em; color:#596574; }
  .row { display:flex; align-items:center; justify-content:space-between; gap:12px; }
  .row.switch { justify-content:flex-start; }
  .slider { display:flex; align-items:center; gap:8px; }
  output { min-width:40px; text-align:right; font-variant-numeric:tabular-nums; color:#596574; }
  input[type='number'] { width:80px; text-align:right; font:inherit; padding:4px 6px; border:1px solid #cbd0d7; border-radius:5px; }
</style>
