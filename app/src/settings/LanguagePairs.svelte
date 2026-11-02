<script lang="ts">
  import { t } from '../lib/i18n';
  import { icons } from '../lib/icons';
  let { pairs, languages, onChange }: { pairs: Record<string, string>; languages: string[]; onChange: (pairs: Record<string, string>) => void } = $props();
  let from = $state('Russian');
  let to = $state('Ukrainian');
  // Codepoint order, like Swift's `keys.sorted()`.
  let keys = $derived(Object.keys(pairs).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)));
  const add = () => onChange({ ...pairs, [from]: to });
  function remove(key: string) {
    const { [key]: _removed, ...rest } = pairs;
    onChange(rest);
  }
</script>
<div class="pairs">
  <strong>{t('Language pairs')}</strong>
  {#if keys.length}
    <ul>
      {#each keys as key (key)}
        <li><span class="from">{key}</span><span aria-hidden="true">→</span><span class="to">{pairs[key]}</span>
          <button type="button" class="plain" title={t('Remove %@', [key])} aria-label={t('Remove %@', [key])} onclick={() => remove(key)}>{@html icons.minus}</button></li>
      {/each}
    </ul>
  {/if}
  <div class="add">
    <select aria-label={t('Detected language')} bind:value={from}>{#each languages as language}<option value={language}>{language}</option>{/each}</select>
    <span aria-hidden="true">→</span>
    <select aria-label={t('Target language')} bind:value={to}>{#each languages as language}<option value={language}>{language}</option>{/each}</select>
    <button type="button" class="plain" title={t('Add language pair')} aria-label={t('Add language pair')} onclick={add}>{@html icons.plus}</button>
  </div>
  <p class="footnote">{t('If detected source has no pair, the default target is used.')}</p>
</div>
<style>
  .pairs { display:flex; flex-direction:column; gap:6px; margin-top:4px; font-size:12px; }
  ul { list-style:none; margin:0; padding:0; display:flex; flex-direction:column; gap:4px; }
  li { display:flex; align-items:center; gap:8px; }
  .from { width:110px; }
  .to { flex:1; }
  .add { display:flex; align-items:center; gap:8px; padding-top:6px; border-top:1px solid #e1e5ea; }
  .add select { width:140px; }
  .plain { padding:4px 6px; }
  .footnote { margin:0; color:#596574; }
</style>
