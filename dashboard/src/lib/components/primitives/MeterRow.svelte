<script lang="ts">
  import { providerName } from "../../copy";
  import { pushEscape } from "../../escape";
  import {
    absolute,
    age,
    countdown,
    meterValueText,
    percent,
    percentDetail,
    timeOfDay,
    windowName,
    type WindowMeterView,
  } from "../../format";
  import type { ProviderKey } from "../../ipc/bindings/ProviderKey";
  import { statusName } from "../../copy";
  import { clock } from "../../stores/clock.svelte";
  import { settings } from "../../stores/settings.svelte";
  import BigNumber from "./BigNumber.svelte";
  import DetailRows from "./DetailRows.svelte";
  import type { DetailRow } from "./display";
  import Meter from "./Meter.svelte";
  import MeterLabel from "./MeterLabel.svelte";
  import StaleRow from "./StaleRow.svelte";

  interface Props {
    provider: ProviderKey;
    meter: WindowMeterView;
    /** When the reading was taken (epoch milliseconds). */
    observedAt: number;
    /** The provider's name on the label line, when no heading above says it. */
    heading?: string;
    /** `medium` for secondary windows and narrow layouts (S15). */
    size?: "large" | "medium";
    /** The latest read failed: this is the last good value (S27). */
    stale?: boolean;
  }

  const {
    provider,
    meter,
    observedAt,
    heading,
    size = "large",
    stale = false,
  }: Props = $props();

  const hour = $derived(settings.value.hour_cycle);
  const name = $derived(windowName(meter.window));
  const label = $derived(`${providerName[provider]} ${name}`);
  const elapsed = $derived(meter.phase === "elapsed");
  const warnAt = $derived(Math.round(settings.value.warn_bp / 100));
  const criticalAt = $derived(Math.round(settings.value.critical_bp / 100));
  const raised = $derived(
    !stale &&
      !elapsed &&
      (meter.severity === "warn" || meter.severity === "critical"),
  );
  const left = $derived(
    meter.resets_at === null ? null : countdown(meter.resets_at, clock.now),
  );
  const lastRead = $derived(
    `Last good read ${age(observedAt, clock.now, hour)}`,
  );
  const valueText = $derived(
    stale
      ? `${meterValueText(meter, clock.now, hour, observedAt)}, stale, ${lastRead.toLowerCase()}`
      : meterValueText(meter, clock.now, hour, observedAt),
  );
  const quiet = $derived.by(() => {
    if (meter.resets_at === null) return "Reset time unknown";
    if (elapsed || left === null) {
      return `Reset ${absolute(meter.resets_at, clock.now, hour)} · waiting for your next request`;
    }
    if (raised) return `Past the ${String(warnAt)}% mark. Resets in ${left}`;
    return `Resets in ${left} · ${absolute(meter.resets_at, clock.now, hour)}`;
  });
  const detail: DetailRow[] = $derived([
    {
      label: "Used",
      value: percentDetail(meter.used_bp),
      alert: raised,
    },
    {
      label: "Exact reset",
      value:
        meter.resets_at === null
          ? "Reset time unknown"
          : absolute(meter.resets_at, clock.now, hour),
    },
    { label: "Status", value: statusName[meter.status] },
    { label: "Seen at", value: timeOfDay(observedAt, hour) },
  ]);

  // Level 1 (DESIGN 9.1): the detail opens on hover or keyboard focus, stays open while the
  // pointer is over it, and closes on Esc without moving focus (WCAG 1.4.13).
  let hovered = $state(false);
  let focused = $state(false);
  let dismissed = $state(false);
  const open = $derived((hovered || focused) && !dismissed);

  $effect(() => {
    if (!open) return;
    return pushEscape("disclosure", () => {
      dismissed = true;
      return true;
    });
  });

  function leave(): void {
    if (!hovered && !focused) dismissed = false;
  }
</script>

<div
  class="row {size}"
  role="presentation"
  onpointerenter={() => {
    hovered = true;
  }}
  onpointerleave={() => {
    hovered = false;
    leave();
  }}
  onfocusin={() => {
    focused = true;
  }}
  onfocusout={(event) => {
    if (
      !(event.relatedTarget instanceof Node) ||
      !event.currentTarget.contains(event.relatedTarget)
    ) {
      focused = false;
      leave();
    }
  }}
>
  <div class="label">
    <MeterLabel
      name={heading}
      window={name}
      status={raised ? "near_limit" : undefined}
    />
  </div>
  <div class="waffle-slot">
    <Meter
      usedBp={meter.used_bp}
      severity={meter.severity}
      {label}
      {valueText}
      {size}
      {stale}
      warnAtPct={warnAt}
      criticalAtPct={criticalAt}
      focusable
    />
  </div>
  <div class="reading">
    <BigNumber
      value={elapsed ? "—" : percent(meter.used_bp)}
      unit={elapsed ? undefined : "used"}
      alert={raised}
    />
    {#if stale}
      <StaleRow text={lastRead} />
    {:else}
      <p class="quiet">{quiet}</p>
    {/if}
  </div>
  {#if open}
    <!-- Opened by the keyboard alone, it skips the hover-intent wait. -->
    <div class="detail" class:instant={focused && !hovered} aria-hidden="true">
      <DetailRows rows={detail} floating />
    </div>
  {/if}
</div>

<style>
  /* S22: a 256 column — label, 16, waffle, 24, number, 4, caption. */
  .row {
    position: relative;
    display: grid;
    align-content: start;
    inline-size: 16rem;
    max-inline-size: 100%;
    padding-block: var(--space-3);
  }
  /* A status badge must not push one column's waffle below its neighbours'. */
  .label {
    min-block-size: var(--space-6);
  }
  .waffle-slot {
    margin-block-start: var(--space-4);
  }
  .reading {
    display: grid;
    gap: var(--space-1);
    margin-block-start: var(--space-6);
  }

  /* S15: with the medium waffle the number sits beside it, never inside. */
  .row.medium {
    grid-template-columns: max-content minmax(0, 1fr);
    column-gap: var(--space-4);
    inline-size: 100%;
  }
  .medium .label {
    grid-column: 1 / -1;
  }
  .medium .reading {
    align-self: center;
    margin-block-start: var(--space-4);
  }

  .quiet {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
    font-variant-numeric: tabular-nums;
  }

  /* S23, floating: the detail opens over the space below the row instead of in flow, so
     opening it moves nothing on the page, and it never covers the meter that opened it.
     The 4 px above it is part of it (transparent), so the pointer can cross onto it and it
     stays open (WCAG 1.4.13). After a short hover intent it unrolls downward out of the row
     on the heavy spring, solid from the first frame (a clip reveal, not a fade), and its
     rows land one after another. Unmounting closes it at once. */
  .detail {
    position: absolute;
    inset-inline: 0;
    top: 100%;
    z-index: 3;
    padding-block-start: var(--space-1);
    transition: translate var(--duration-spring-heavy) var(--ease-spring-heavy)
      var(--duration-intent);
  }
  .detail :global(dl) {
    clip-path: inset(0 round var(--radius-control));
    transition: clip-path var(--duration-spring-heavy) var(--ease-spring-heavy)
      var(--duration-intent);
  }
  /* Rows land one after another (the CSP allows no inline style, so no index variable). */
  .detail :global(dl > div) {
    transition:
      opacity var(--duration-fast) var(--ease) var(--row-delay),
      translate var(--duration-spring-firm) var(--ease-spring-firm)
        var(--row-delay);
    --row-delay: calc(var(--duration-intent) + var(--duration-stagger) * 2);
  }
  .detail :global(dl > div:nth-child(2)) {
    --row-delay: calc(var(--duration-intent) + var(--duration-stagger) * 3);
  }
  .detail :global(dl > div:nth-child(3)) {
    --row-delay: calc(var(--duration-intent) + var(--duration-stagger) * 4);
  }
  .detail :global(dl > div:nth-child(n + 4)) {
    --row-delay: calc(var(--duration-intent) + var(--duration-stagger) * 5);
  }
  .instant,
  .instant :global(dl) {
    transition-delay: 0s;
  }
  @starting-style {
    .detail {
      translate: 0 calc(var(--space-2) * -1);
    }
    .detail :global(dl) {
      clip-path: inset(0 0 100% 0 round var(--radius-control));
    }
    .detail :global(dl > div) {
      opacity: 0;
      translate: 0 calc(var(--space-1) * -1.5);
    }
  }
</style>
