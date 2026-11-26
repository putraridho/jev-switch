<script lang="ts">
  import {
    buildChart,
    momentLabel,
    nearest,
    readout,
    stepIndex,
    summary,
    valueLabel,
    type Thresholds,
  } from "../../chart";
  import { money } from "../../format";
  import type { Currency } from "../../ipc/bindings/Currency";
  import type { HistoryRange } from "../../ipc/bindings/HistoryRange";
  import type { HistorySeries } from "../../ipc/bindings/HistorySeries";
  import { clock } from "../../stores/clock.svelte";
  import { settings } from "../../stores/settings.svelte";

  interface Props {
    series: HistorySeries;
    range: HistoryRange;
    /** The meter's name for the summary: "5-hour". */
    name: string;
    /** Warn and critical lines; `null` for a balance. */
    thresholds: Thresholds;
    /** The currency of a balance series. */
    currency: Currency | null;
  }

  const { series, range, name, thresholds, currency }: Props = $props();

  const uid = $props.id();
  const hour = $derived(settings.value.hour_cycle);
  const word = (micros: number): string =>
    currency === null
      ? (micros / 1_000_000).toFixed(2)
      : money(micros, currency);

  let width = $state(480);
  const model = $derived(
    buildChart(series, thresholds, range, hour, clock.now, word, width),
  );
  const description = $derived(
    summary(series, name, range, hour, clock.now, word),
  );

  // The crosshair is one control: a slider over the plot. Keys and the pointer move the same
  // index, and its value text is the readout (a focused slider is spoken by the screen
  // reader itself, so this does not go through the live region).
  let picked = $state<number | null>(null);
  let hovered = $state<number | null>(null);
  const count = $derived(series.points.length);
  const shown = $derived(hovered ?? picked);
  const shownPoint = $derived(shown === null ? undefined : model.points[shown]);
  const valueText = $derived(
    shownPoint === undefined
      ? ""
      : readout(series, shownPoint.point, range, hour, clock.now, word),
  );
  // The slider's own value follows the keyboard position, not a passing pointer.
  const pickedPoint = $derived(
    picked === null ? undefined : series.points[picked],
  );
  const pickedText = $derived(
    pickedPoint === undefined
      ? ""
      : readout(series, pickedPoint, range, hour, clock.now, word),
  );
  const rows = $derived(
    series.points.map((point) => ({
      t: point.t,
      when: momentLabel(point.t, range, hour, clock.now),
      value: valueLabel(series, point.v, word),
    })),
  );

  function onkeydown(event: KeyboardEvent): void {
    const from = picked ?? count - 1;
    const next = stepIndex(event.key, from, count);
    if (next === null) return;
    event.preventDefault();
    picked = next;
    hovered = null;
  }

  let focused = $state(false);
  function onfocus(): void {
    focused = true;
    picked ??= count - 1;
  }

  function onpointermove(event: PointerEvent): void {
    const box = event.currentTarget;
    if (!(box instanceof Element)) return;
    const rect = box.getBoundingClientRect();
    hovered = nearest(model.points, frame.left + event.clientX - rect.left);
  }

  const frame = $derived(model.frame);
  const plotWidth = $derived(frame.width - frame.left - frame.right);
  const plotHeight = $derived(frame.height - frame.top - frame.bottom);
</script>

<figure bind:clientWidth={width}>
  <svg
    role="img"
    aria-label={description}
    width={frame.width}
    height={frame.height}
    viewBox="0 0 {frame.width} {frame.height}"
  >
    {#each model.yTicks as tick (tick.at)}
      <line
        class="grid"
        x1={frame.left}
        x2={frame.width - frame.right}
        y1={tick.at}
        y2={tick.at}
      />
      <text
        class="axis"
        x={frame.left - 6}
        y={tick.at}
        text-anchor="end"
        dominant-baseline="middle">{tick.label}</text
      >
    {/each}
    {#each model.xTicks as tick, i (tick.at)}
      <text
        class="axis"
        x={tick.at}
        y={frame.height - 8}
        text-anchor={i === 0
          ? "start"
          : i === model.xTicks.length - 1
            ? "end"
            : "middle"}>{tick.label}</text
      >
    {/each}
    {#each model.resets as x (x)}
      <line
        class="reset"
        x1={x}
        x2={x}
        y1={frame.top}
        y2={frame.height - frame.bottom}
      />
    {/each}
    {#each model.thresholds as line (line.kind)}
      <line
        class="threshold {line.kind}"
        x1={frame.left}
        x2={frame.width - frame.right}
        y1={line.y}
        y2={line.y}
      />
      <text
        class="threshold-word {line.kind}"
        x={frame.width - frame.right - 2}
        y={line.y - 4}
        text-anchor="end">{line.label}</text
      >
    {/each}
    {#if model.band !== null}<path class="band" d={model.band} />{/if}
    <path class="area" d={model.area} />
    <path class="line" d={model.line} />
    {#if shownPoint !== undefined}
      <line
        class="cross"
        x1={shownPoint.x}
        x2={shownPoint.x}
        y1={frame.top}
        y2={frame.top + plotHeight}
      />
      <circle class="dot" cx={shownPoint.x} cy={shownPoint.y} r="4" />
    {/if}
  </svg>
  {#if count > 0}
    <!-- The crosshair is its own control, not inside the described image (WCAG 4.1.2). -->
    <svg
      class="layer"
      width={frame.width}
      height={frame.height}
      viewBox="0 0 {frame.width} {frame.height}"
    >
      <rect
        class="inspect"
        class:focused
        role="slider"
        tabindex="0"
        aria-label="Inspect history"
        aria-valuemin="0"
        aria-valuemax={count - 1}
        aria-valuenow={picked ?? count - 1}
        aria-valuetext={pickedText}
        x={frame.left}
        y={frame.top}
        width={plotWidth}
        height={plotHeight}
        {onkeydown}
        {onfocus}
        onblur={() => {
          focused = false;
        }}
        {onpointermove}
        onpointerdown={() => {
          picked = hovered ?? picked;
        }}
        onpointerleave={() => {
          hovered = null;
        }}
      />
    </svg>
  {/if}
  <p class="readout" aria-hidden="true">
    {valueText === ""
      ? "Hover or focus the chart, then use the arrow keys."
      : valueText}
  </p>
  <div class="visually-hidden">
    <table id="{uid}-table">
      <caption>{description}</caption>
      <thead><tr><th scope="col">Time</th><th scope="col">Value</th></tr></thead
      >
      <tbody>
        {#each rows as row (row.t)}
          <tr><td>{row.when}</td><td>{row.value}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</figure>

<style>
  figure {
    position: relative;
    margin: var(--space-4) 0 0;
  }

  svg {
    display: block;
    overflow: visible;
  }

  /* The control layer sits exactly over the picture. */
  .layer {
    position: absolute;
    top: 0;
    left: 0;
  }

  .grid {
    stroke: var(--color-rule);
    stroke-width: 1;
  }

  .axis {
    fill: var(--color-muted);
    font-family: var(--font-sans);
    font-size: var(--text-micro);
    font-variant-numeric: tabular-nums;
  }

  .reset {
    stroke: var(--color-faint);
    stroke-width: 1;
  }

  .threshold {
    stroke-width: 1.5;
    stroke-dasharray: 5 4;
  }

  .threshold.warn {
    stroke: var(--color-stale);
  }

  .threshold.critical {
    stroke: var(--color-alert);
  }

  /* A paper-coloured halo keeps the words clear of the gridline and the crosshair. */
  .threshold-word {
    font-family: var(--font-sans);
    font-size: var(--text-micro);
    paint-order: stroke;
    stroke: var(--color-paper);
    stroke-width: 4px;
    stroke-linejoin: round;
  }

  .threshold-word.warn {
    fill: var(--color-stale);
  }

  .threshold-word.critical {
    fill: var(--color-alert);
  }

  .band {
    fill: var(--color-ink);
    fill-opacity: 0.14;
    stroke: none;
  }

  .area {
    fill: var(--color-ink);
    fill-opacity: 0.07;
    stroke: none;
  }

  .line {
    fill: none;
    stroke: var(--color-ink);
    stroke-width: 2;
    stroke-linejoin: round;
  }

  .cross {
    stroke: var(--color-ink);
    stroke-width: 1;
  }

  .dot {
    fill: var(--color-paper);
    stroke: var(--color-ink);
    stroke-width: 2;
  }

  /* The slider is the plot, transparent; a drawn ring shows its focus (SVG outlines are
     unreliable in WebKit). */
  .inspect {
    fill: transparent;
    stroke: none;
    cursor: crosshair;
    outline: none;
  }

  .inspect.focused {
    stroke: var(--color-accent);
    stroke-width: 2;
  }

  .readout {
    min-height: var(--text-sm-line);
    margin-top: var(--space-2);
    font-variant-numeric: tabular-nums;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  /* Forced colours: the system picks the colours; the marks keep their shapes (dashes for
     thresholds, hairlines for resets) and draw in the forced text colour. */
  @media (forced-colors: active) {
    .line,
    .cross,
    .dot,
    .grid,
    .reset,
    .threshold,
    .inspect.focused {
      stroke: currentColor;
    }
    .area,
    .band,
    .axis,
    .threshold-word {
      fill: currentColor;
    }
    .threshold-word {
      stroke: none;
    }
  }
</style>
