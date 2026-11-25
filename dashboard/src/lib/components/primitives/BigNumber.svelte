<script lang="ts">
  interface Props {
    /** The figure as text: "62%", "$18.40", "—". */
    value: string;
    /** The small unit word on the same baseline (S19). */
    unit?: string | undefined;
    /** Past the warning mark: the figure turns alert. Never set while stale. */
    alert?: boolean;
  }

  const { value, unit, alert = false }: Props = $props();
</script>

<p class="figure">
  <span class="value" class:alert>{value}</span>
  {#if unit !== undefined}<span class="unit">{unit}</span>{/if}
</p>

<style>
  .figure {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    column-gap: var(--space-2);
    margin: 0;
  }
  .value {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
    font-weight: var(--num-xl-weight);
    font-variant-numeric: tabular-nums;
    color: var(--color-ink);
  }
  .value.alert {
    color: var(--color-alert);
  }
  .unit {
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
  }
</style>
