<!-- S48: a 96 by 24 trace of recent use. Ink line, one dot at the end, alert colour only
     past the warning mark. It is decoration: the number beside it carries the value. -->
<script lang="ts">
  import type { HistorySeries } from "../../ipc/bindings/HistorySeries";
  import { buildTrace, SPARK } from "./sparkline";

  interface Props {
    /** `null`, or a series without points, draws the dotted "no data" baseline. */
    series: HistorySeries | null;
    /** The warning mark in percent. */
    warnAtPct?: number;
  }

  const { series, warnAtPct = 80 }: Props = $props();

  const trace = $derived(
    series === null ? null : buildTrace(series, warnAtPct),
  );
</script>

<svg
  class="spark"
  viewBox="0 0 {SPARK.width} {SPARK.height}"
  width={SPARK.width}
  height={SPARK.height}
  aria-hidden="true"
>
  {#if trace === null}
    <path
      class="empty"
      d="M2 {SPARK.height / 2} L{SPARK.width - 2} {SPARK.height / 2}"
    />
  {:else}
    {#if trace.guide !== null}
      <line
        class="guide"
        x1="0"
        x2={SPARK.width}
        y1={trace.guide}
        y2={trace.guide}
      />
    {/if}
    {#if trace.ink !== ""}<path class="ink" d={trace.ink} />{/if}
    {#if trace.alert !== ""}<path class="alert" d={trace.alert} />{/if}
    <circle
      class="end"
      class:raised={trace.end.alert}
      cx={trace.end.x}
      cy={trace.end.y}
      r="3"
    />
  {/if}
</svg>

<style>
  .spark {
    display: block;
    flex-shrink: 0;
  }
  path {
    fill: none;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .ink {
    stroke: var(--color-ink);
  }
  .alert {
    stroke: var(--color-alert);
  }
  .empty {
    stroke: var(--color-faint);
    stroke-dasharray: 0.1 5;
  }
  .guide {
    stroke: var(--color-rule);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }
  .end {
    fill: var(--color-ink);
  }
  .end.raised {
    fill: var(--color-alert);
  }
  @media (forced-colors: active) {
    path,
    .guide {
      stroke: currentColor;
    }
    .end,
    .end.raised {
      fill: currentColor;
    }
  }
</style>
