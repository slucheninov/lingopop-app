<script lang="ts">
  import { t } from '../lib/i18n';
  import type { AppLanguage, SettingsPatch, SettingsView, UiPatch } from '../lib/settingsTypes';
  import LanguagePairs from './LanguagePairs.svelte';
  import LoginItemNotice from './LoginItemNotice.svelte';
  import PermissionsSection from './PermissionsSection.svelte';
  import Relaunch from './Relaunch.svelte';
  let { view, onPatch, onUiPatch }: { view: SettingsView; onPatch: (patch: SettingsPatch) => Promise<boolean>; onUiPatch: (patch: UiPatch) => Promise<boolean> } = $props();

  // Native names are never translated; `System` is.
  const APP_LANGUAGES: { value: AppLanguage; name: string | null }[] = [
    { value: 'system', name: null }, { value: 'en', name: 'English' }, { value: 'pl', name: 'Polski' },
    { value: 'uk', name: 'Українська' }, { value: 'de', name: 'Deutsch' },
  ];
  let targetOptions = $derived(view.quickLanguages.includes(view.targetLanguage) ? view.quickLanguages : [view.targetLanguage, ...view.quickLanguages]);

  // The slider shows the dragged value at once and follows the saved one when it changes from outside.
  let fontSize = $state(0);
  let sentFontSize = 0;
  // While a font save is in flight the control keeps what the user dragged to (a view for an older commit may arrive).
  let fontInFlight = 0;
  $effect(() => { const saved = view.popupFontSize; if (fontInFlight === 0) { fontSize = saved; sentFontSize = saved; } });
  function onFont(event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(value)) return;
    fontSize = value;
    if (value === sentFontSize) return;
    sentFontSize = value;
    fontInFlight++;
    void onUiPatch({ field: 'popupFontSize', value }).then(saved => {
      fontInFlight--;
      // A refused save puts the control back on the saved value and forgets the dedup state, so a retry is sent.
      if (!saved && fontInFlight === 0) { fontSize = view.popupFontSize; sentFontSize = view.popupFontSize; }
    });
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
    void onPatch({ field, value }).then(saved => { if (!saved) input.value = String(view[field]); });
  }
  /** Puts a switch / select back on the saved value when the shell refused the change. */
  function revert(saved: Promise<boolean>, restore: () => void) { void saved.then(ok => { if (!ok) restore(); }); }
</script>
<div class="main">
  {#if view.platform === 'macos'}<PermissionsSection />{/if}
  <section>
    <h3>{t('Target language')}</h3>
    <label class="row switch"><input type="checkbox" role="switch" checked={view.useAutoDetect} onchange={event => { const box = event.currentTarget; revert(onPatch({ field: 'useAutoDetect', value: box.checked }), () => { box.checked = view.useAutoDetect; }); }} /><span>{t('Auto-detect input language')}</span></label>
    <label class="row"><span>{view.useAutoDetect ? t('Default target') : t('Target')}</span>
      <select value={view.targetLanguage} onchange={event => { const select = event.currentTarget; revert(onPatch({ field: 'targetLanguage', value: select.value }), () => { select.value = view.targetLanguage; }); }}>
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
      <select value={view.appLanguage} onchange={event => { const select = event.currentTarget; revert(onUiPatch({ field: 'appLanguage', value: select.value as AppLanguage }), () => { select.value = view.appLanguage; }); }}>
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
    <label class="row switch"><input type="checkbox" role="switch" checked={view.launchAtLogin} onchange={event => { const box = event.currentTarget; revert(onPatch({ field: 'launchAtLogin', value: box.checked }), () => { box.checked = view.launchAtLogin; }); }} /><span>{t('Launch at login')}</span></label>
    {#if view.loginItemIssue}<LoginItemNotice issue={view.loginItemIssue} />{/if}
    <label class="row switch"><input type="checkbox" role="switch" checked={view.autoCopyOCR} onchange={event => { const box = event.currentTarget; revert(onPatch({ field: 'autoCopyOCR', value: box.checked }), () => { box.checked = view.autoCopyOCR; }); }} /><span>{t('Auto-copy OCR/translation results to clipboard')}</span></label>
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
