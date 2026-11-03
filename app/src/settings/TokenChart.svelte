<script lang="ts">
  import type { UsageBucket } from '../lib/settingsTypes';
  let { buckets, label }: { buckets: UsageBucket[]; label: string } = $props();

  const CHART_HEIGHT = 60;
  const GAP = 3;
  // 1 user unit = 1 CSS px once the plot is measured, so the gap stays 3 px.
  // jsdom has no ResizeObserver; the 100-wide fallback keeps bar heights testable.
  let plotWidth = $state(0);
  let chartWidth = $derived(plotWidth > 0 ? plotWidth : 100);
  let plotEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    const el = plotEl;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(entries => {
      const width = entries[0]?.contentRect.width ?? 0;
      if (width > 0) plotWidth = width;
    });
    observer.observe(el);
    return () => observer.disconnect();
  });
  const TRACKED = ['Claude', 'OpenAI', 'Gemini', 'Grok'] as const;
  const FILL: Record<(typeof TRACKED)[number], string> = {
    Claude: '#ff3b30', OpenAI: '#34c759', Gemini: '#007aff', Grok: '#af52de',
  };

  function bucketTotal(bucket: UsageBucket): number {
    return Object.values(bucket.byProvider).reduce((sum, pair) => sum + pair[0] + pair[1], 0);
  }

  let bars = $derived.by(() => {
    const max = Math.max(1, ...buckets.map(bucketTotal));
    const count = buckets.length;
    const barWidth = count === 0 ? 0 : (chartWidth - GAP * (count - 1)) / count;
    const drawn: { x: number; y: number; width: number; height: number; provider: string; fill: string }[] = [];
    buckets.forEach((bucket, index) => {
      const x = index * (barWidth + GAP);
      let y = CHART_HEIGHT;
      for (const provider of TRACKED) {
        const pair = bucket.byProvider[provider];
        const total = pair ? pair[0] + pair[1] : 0;
        if (total <= 0) continue;
        const height = (total * CHART_HEIGHT) / max;
        y -= height;
        drawn.push({ x, y, width: Math.max(barWidth, 1), height, provider, fill: FILL[provider] });
      }
    });
    return drawn;
  });
</script>
<div class="chart">
  <div class="plot" bind:this={plotEl}>
  <svg role="img" aria-label={label} height="60" viewBox="0 0 {chartWidth} {CHART_HEIGHT}">
    {#each bars as bar (`${bar.provider}-${bar.x}`)}
      <rect x={bar.x} y={bar.y} width={bar.width} height={bar.height} rx="2" fill={bar.fill} data-provider={bar.provider} />
    {/each}
  </svg>
  </div>
  <div class="axis" aria-hidden="true">
    {#each buckets as bucket, index (bucket.startUnix + ':' + index)}
      <span>{bucket.label}</span>
    {/each}
  </div>
  <ul class="legend">
    {#each TRACKED as provider (provider)}
      <li><span class="swatch" style:background={FILL[provider]}></span>{provider}</li>
    {/each}
  </ul>
  <table class="sr-only">
    <tbody>
      {#each buckets as bucket, index (bucket.startUnix + ':' + index)}
        <tr><th>{bucket.label}</th><td>{bucketTotal(bucket)}</td></tr>
      {/each}
    </tbody>
  </table>
</div>
<style>
  .plot { width: 100%; }
  svg { width: 100%; height: 60px; display: block; }
  .axis { display: flex; gap: 3px; height: 14px; margin-top: 4px; }
  .axis span { flex: 1; min-width: 0; overflow: hidden; text-align: center; font-size: 10px; color: #8b95a1; }
  .legend { display: flex; gap: 12px; margin: 6px 0 0; padding: 0; list-style: none; font-size: 11px; color: #596574; }
  .swatch { display: inline-block; width: 10px; height: 10px; margin-right: 4px; border-radius: 2px; vertical-align: -1px; }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
</style>
