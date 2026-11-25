<script lang="ts">
  import { checkClaudeNow } from "../../actions";
  import { rangeNames } from "../../chart";
  import { sourceLine } from "../../copy";
  import { windowName } from "../../format";
  import type { HistoryRange } from "../../ipc/bindings/HistoryRange";
  import type { HistorySeries } from "../../ipc/bindings/HistorySeries";
  import type { MeterView } from "../../ipc/bindings/MeterView";
  import type { ProviderKey } from "../../ipc/bindings/ProviderKey";
  import { queries } from "../../stores/queries";
  import { settings } from "../../stores/settings.svelte";
  import HistoryChart from "../primitives/HistoryChart.svelte";

  interface Props {
    provider: ProviderKey;
    meters: readonly MeterView[];
  }

  const { provider, meters }: Props = $props();

  const ranges: readonly HistoryRange[] = ["day", "week", "month", "quarter"];
  const uid = $props.id();

  let range = $state<HistoryRange>("day");
  let chosen = $state<string | null>(null);
  const meter = $derived(
    meters.find((item) => item.id === chosen) ?? meters[0],
  );

  function nameOf(item: MeterView): string {
    return item.kind === "window" ? windowName(item.window) : "Balance";
  }

  type Loaded =
    | { state: "loading" }
    | { state: "error" }
    | { state: "ready"; series: HistorySeries };
  let loaded = $state<Loaded>({ state: "loading" });
  let latest = 0;

  // Only the newest question may answer: a slow reply for another range or meter is dropped.
  async function load(meterId: string, wanted: HistoryRange): Promise<void> {
    latest += 1;
    const mine = latest;
    loaded = { state: "loading" };
    const result = await queries.history({
      provider,
      meter_id: meterId,
      range: wanted,
    });
    if (mine !== latest) return;
    loaded =
      result.status === "ok"
        ? { state: "ready", series: result.value }
        : { state: "error" };
  }

  $effect(() => {
    const id = meter?.id;
    if (id !== undefined) void load(id, range);
  });

  const thresholds = $derived(
    meter?.kind === "window"
      ? {
          warnBp: settings.value.warn_bp,
          criticalBp: settings.value.critical_bp,
        }
      : null,
  );
  const currency = $derived(meter?.kind === "balance" ? meter.currency : null);
</script>

<section aria-labelledby="{uid}-title">
  <h2 id="{uid}-title">History</h2>
  <div class="controls">
    {#if meters.length > 1}
      <label class="pick"
        >Meter
        <select
          value={meter?.id}
          onchange={(event) => {
            chosen = event.currentTarget.value;
          }}
        >
          {#each meters as item (item.id)}
            <option value={item.id}>{nameOf(item)}</option>
          {/each}
        </select>
      </label>
    {/if}
    <fieldset>
      <legend>Range</legend>
      {#each ranges as option (option)}
        <label class:on={range === option}>
          <input
            type="radio"
            name="{uid}-range"
            value={option}
            checked={range === option}
            onchange={() => {
              range = option;
            }}
          />
          {rangeNames[option]}
        </label>
      {/each}
    </fieldset>
  </div>
  {#if loaded.state === "loading"}
    <p class="state">Loading history…</p>
  {:else if loaded.state === "error"}
    <p class="state">History isn't available right now.</p>
    <button
      type="button"
      onclick={() => {
        if (meter !== undefined) void load(meter.id, range);
      }}>Retry</button
    >
  {:else if loaded.series.points.length === 0}
    <p class="state">No history in this range yet.</p>
  {:else if meter !== undefined}
    <HistoryChart
      series={loaded.series}
      {range}
      name={nameOf(meter)}
      {thresholds}
      {currency}
    />
  {/if}
  <p class="source">{sourceLine(provider)}</p>
  {#if provider === "claude" && settings.value.claude_live_check}
    <div class="check">
      <button type="button" onclick={checkClaudeNow}>Check now</button>
      <p>Uses one small request.</p>
    </div>
  {/if}
</section>

<style>
  h2 {
    margin-top: var(--space-8);
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
  }

  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-4);
    margin-top: var(--space-2);
  }

  .pick {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  fieldset {
    display: flex;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }

  fieldset label {
    display: flex;
    align-items: center;
    min-height: var(--hit-min);
    padding-inline: var(--space-3);
    border: var(--outline) solid var(--color-border);
    border-radius: var(--radius-control);
    font-size: var(--text-sm);
    cursor: pointer;
  }

  /* The chosen range is marked by weight and an ink border, not by colour alone. */
  fieldset label.on {
    border-color: var(--color-ink);
    font-weight: var(--text-sm-weight-strong);
  }

  fieldset label:has(:focus-visible) {
    outline: var(--focus-ring) solid var(--color-accent);
    outline-offset: 2px;
  }

  input[type="radio"] {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
  }

  .state {
    margin-block: var(--space-4);
    min-height: 6rem;
  }

  .source {
    margin-top: var(--space-6);
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }
</style>
