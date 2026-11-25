<script lang="ts">
  import { providerName } from "../../copy";
  import { pushEscape } from "../../escape";
  import {
    absolute,
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
  import Meter from "./Meter.svelte";

  interface Props {
    provider: ProviderKey;
    meter: WindowMeterView;
    /** When the reading was taken (epoch milliseconds). */
    observedAt: number;
  }

  const { provider, meter, observedAt }: Props = $props();

  const hour = $derived(settings.value.hour_cycle);
  const name = $derived(windowName(meter.window));
  const label = $derived(`${providerName[provider]} ${name}`);
  const elapsed = $derived(meter.phase === "elapsed");
  const left = $derived(
    meter.resets_at === null ? null : countdown(meter.resets_at, clock.now),
  );
  const valueText = $derived(
    meterValueText(meter, clock.now, hour, observedAt),
  );
  const quiet = $derived.by(() => {
    if (meter.resets_at === null) return "Reset time unavailable";
    if (elapsed || left === null) {
      return `Reset ${absolute(meter.resets_at, clock.now, hour)} · waiting for your next request`;
    }
    return `Resets in ${left}`;
  });

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
  class="row"
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
  <p class="window">{name}</p>
  <div class="value">{elapsed ? "—" : percent(meter.used_bp)}</div>
  <div class="waffle-slot">
    <Meter
      usedBp={meter.used_bp}
      severity={meter.severity}
      {label}
      {valueText}
      focusable
    />
  </div>
  <p class="quiet">{quiet}</p>
  {#if open}
    <div class="detail" aria-hidden="true">
      <p>{percentDetail(meter.used_bp)} used</p>
      <p>
        {meter.resets_at === null
          ? "Reset time unavailable"
          : `Resets ${absolute(meter.resets_at, clock.now, hour)}`}
      </p>
      <p>Status: {statusName[meter.status]}</p>
      <p>Seen at {timeOfDay(observedAt, hour)}</p>
    </div>
  {/if}
</div>

<style>
  .row {
    position: relative;
    padding-block: var(--space-3);
  }

  /* With room (the 10x10 waffle is 240 px wide at its 24 px pitch) the number and the quiet
     line sit beside it, as in the wireframes; narrower, everything stacks. The DOM order is
     the reading order either way: name, number, meter, quiet line. */
  @media (min-width: 34rem) {
    .row {
      display: grid;
      grid-template-columns: 1fr max-content;
      grid-template-rows: auto auto auto 1fr;
      grid-template-areas:
        "window waffle"
        "value waffle"
        "quiet waffle"
        ". waffle";
      column-gap: var(--space-6);
    }
    .window {
      grid-area: window;
    }
    .value {
      grid-area: value;
    }
    .waffle-slot {
      grid-area: waffle;
    }
    .quiet {
      grid-area: quiet;
    }
  }

  .window {
    font-size: var(--text-sm);
    font-weight: var(--text-sm-weight-strong);
  }

  .value {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
    font-variant-numeric: tabular-nums;
  }

  /* Opens below the row: it never covers the meter that opened it. */
  .detail {
    position: absolute;
    inset-inline: 0;
    top: 100%;
    z-index: 1;
    display: grid;
    gap: var(--space-1);
    padding: var(--space-3);
    background: var(--color-surface-2);
    border: var(--outline) solid var(--color-border);
    border-radius: var(--radius-control);
  }

  .detail p {
    color: var(--color-text);
    font-variant-numeric: tabular-nums;
  }
</style>
