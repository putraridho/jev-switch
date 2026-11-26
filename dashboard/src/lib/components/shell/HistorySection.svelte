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
  import EmptyState from "../primitives/EmptyState.svelte";
  import ErrorState from "../primitives/ErrorState.svelte";
  import HistoryChart from "../primitives/HistoryChart.svelte";
  import LoadingWaffle from "../primitives/LoadingWaffle.svelte";
  import RangePills from "../primitives/RangePills.svelte";
  import Section from "../primitives/Section.svelte";
  import SelectField from "../primitives/SelectField.svelte";

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

<Section label="History">
  <div class="controls">
    {#if meters.length > 1}
      <SelectField
        id="{uid}-meter"
        label="Meter"
        value={meter?.id}
        options={meters.map((item) => ({
          value: item.id,
          label: nameOf(item),
        }))}
        onchange={(value: string) => {
          chosen = value;
        }}
      />
    {/if}
    <RangePills
      legend="Range"
      options={ranges.map((value) => ({ value, label: rangeNames[value] }))}
      value={range}
      onchange={(value: HistoryRange) => {
        range = value;
      }}
    />
  </div>
  <div class="plot">
    {#if loaded.state === "loading"}
      <LoadingWaffle size="medium" announcement="Loading history…" />
    {:else if loaded.state === "error"}
      {#snippet again()}<button
          type="button"
          onclick={() => {
            if (meter !== undefined) void load(meter.id, range);
          }}>Retry</button
        >{/snippet}
      <ErrorState message="History isn't available right now." action={again} />
    {:else if loaded.series.points.length === 0}
      <EmptyState
        title="No history in this range yet."
        body="It fills in as Flashboard reads this provider."
        ghost
      />
    {:else if meter !== undefined}
      <HistoryChart
        series={loaded.series}
        {range}
        name={nameOf(meter)}
        {thresholds}
        {currency}
      />
    {/if}
  </div>
  <p class="source">{sourceLine(provider)}</p>
  {#if provider === "claude" && settings.value.claude_live_check}
    <div class="check">
      <button type="button" onclick={checkClaudeNow}>Check now</button>
      <p>Uses one small request.</p>
    </div>
  {/if}
</Section>

<style>
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .plot {
    min-height: 8rem;
    margin-block-start: var(--space-4);
  }

  .source {
    margin-top: var(--space-6);
    color: var(--color-muted);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }

  .check {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }
</style>
