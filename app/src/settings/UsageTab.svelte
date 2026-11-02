<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { t } from '../lib/i18n';
  import { getUsage, resetUsage } from '../lib/settingsIpc';
  import type { SettingsPatch, SettingsView, UiPatch, UsagePeriod, UsageView } from '../lib/settingsTypes';
  import TokenChart from './TokenChart.svelte';

  // The frame passes the settings props to every tab. Usage reads its own command.
  let _props: { view: SettingsView; onPatch: (patch: SettingsPatch) => void; onUiPatch: (patch: UiPatch) => void } = $props();

  const PERIODS: { id: UsagePeriod; label: string }[] = [
    { id: 'day', label: '1 day' },
    { id: 'days7', label: '7 days' },
    { id: 'days30', label: '30 days' },
  ];

  let period = $state<UsagePeriod>('days30');
  let usage = $state<UsageView | null>(null);
  let confirmReset = $state(false);
  let disposed = false;

  async function load() {
    const requested = period;
    try {
      const next = await getUsage(requested);
      if (!disposed && period === requested) usage = next;
    } catch {
      // A rejection is a catalog key. Keep the last view and do not log it.
    }
  }

  function choose(next: UsagePeriod) {
    if (period === next) return;
    period = next;
    void load();
  }

  async function remove() {
    confirmReset = false;
    try { await resetUsage(); } catch { /* the store hook refetches when the reset landed */ }
  }

  onMount(() => {
    const registration = listen('usage://changed', () => { if (!disposed) void load(); });
    void load();
    return () => {
      disposed = true;
      void registration.then(unlisten => unlisten()).catch(() => {});
    };
  });
</script>
<div class="usage">
  <div class="toolbar">
    <span class="caption">{t('Period')}</span>
    <div class="segments" role="group" aria-label={t('Period')}>
      {#each PERIODS as item (item.id)}
        <button type="button" aria-pressed={period === item.id} onclick={() => choose(item.id)}>{t(item.label)}</button>
      {/each}
    </div>
    <button type="button" class="reset" onclick={() => { confirmReset = true; }}>{t('Reset stats…')}</button>
  </div>
  {#if usage}
    <div class="cards">
      <article>
        <h3>{t('Est. Cost')}</h3>
        <p class="value cost">{usage.costLabel}</p>
        <p class="sub">{usage.periodLabel}</p>
      </article>
      <article>
        <h3>{t('Tokens')}</h3>
        <p class="value">{usage.tokensLabel}</p>
        <p class="sub">{usage.inOutLabel}</p>
      </article>
      <article>
        <h3>{t('Calls')}</h3>
        <p class="value">{usage.totals.calls}</p>
        <p class="sub">{usage.callsLabel}</p>
      </article>
    </div>
    <section class="box">
      <h3>{usage.chartTitle}</h3>
      <TokenChart buckets={usage.buckets} label={usage.chartTitle} />
    </section>
    <section class="box">
      <table>
        <thead>
          <tr>
            <th>{t('Provider')}</th>
            <th>{t('Input')}</th>
            <th>{t('Output')}</th>
            <th>{t('Calls')}</th>
            <th class="end">{t('Est. Cost')}</th>
          </tr>
        </thead>
        <tbody>
          {#each usage.rows as row (row.provider)}
            <tr>
              <td class="provider"><span class="dot {row.colour}" aria-hidden="true"></span>{row.name}</td>
              <td class="num">{row.inputLabel}</td>
              <td class="num">{row.outputLabel}</td>
              <td class="num">{row.calls}</td>
              <td class="num end {row.colour}">{row.costLabel}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}
  {#if confirmReset}
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="usage-reset-title">
      <p id="usage-reset-title">{t('Delete all usage history?')}</p>
      <div class="actions">
        <button type="button" onclick={() => { confirmReset = false; }}>{t('Cancel')}</button>
        <button type="button" class="danger" onclick={() => { void remove(); }}>{t('Delete')}</button>
      </div>
    </div>
  {/if}
</div>
<style>
  .usage { display: flex; flex-direction: column; gap: 12px; }
  .toolbar { display: flex; align-items: center; gap: 10px; }
  .caption, h3 { margin: 0; font-size: 11px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; color: #596574; }
  .segments { display: flex; width: 240px; }
  .segments button { flex: 1; border-radius: 0; }
  .segments button + button { margin-left: -1px; }
  .segments button:first-child { border-radius: 5px 0 0 5px; }
  .segments button:last-child { border-radius: 0 5px 5px 0; }
  .segments button[aria-pressed='true'] { background: #3478cf; color: #fff; border-color: #3478cf; position: relative; }
  .reset { margin-left: auto; border: 0; background: none; color: #596574; }
  .cards { display: flex; gap: 10px; }
  article, .box { flex: 1; background: #fff; border: 1px solid #e1e5ea; border-radius: 8px; padding: 10px 12px; }
  .value { margin: 3px 0 0; font-size: 22px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .value.cost { color: #ff3b30; }
  .sub { margin: 3px 0 0; font-size: 11px; color: #8b95a1; }
  .box { display: flex; flex-direction: column; gap: 8px; }
  table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th { text-align: left; font-size: 11px; font-weight: 600; color: #596574; padding: 0 0 6px; }
  td { padding: 8px 0; border-top: 1px solid #e8ebef; }
  .end { text-align: right; }
  .provider { display: flex; align-items: center; gap: 6px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; }
  .red { color: #ff3b30; } .dot.red { background: #ff3b30; }
  .green { color: #34c759; } .dot.green { background: #34c759; }
  .blue { color: #007aff; } .dot.blue { background: #007aff; }
  .purple { color: #af52de; } .dot.purple { background: #af52de; }
  .dialog { display: flex; flex-direction: column; gap: 10px; padding: 12px; background: #fff; border: 1px solid #e1e5ea; border-radius: 8px; }
  .dialog p { margin: 0; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .danger { color: #fff; background: #ff3b30; border-color: #ff3b30; }
</style>
