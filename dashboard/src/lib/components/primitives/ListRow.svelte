<script lang="ts">
  import type { Snippet } from "svelte";
  import { percent } from "../../format";
  import type { Severity } from "../../ipc/bindings/Severity";
  import Meter from "./Meter.svelte";

  interface Props {
    name: string;
    /** The window, e.g. "weekly". */
    window?: string;
    /** `null` when there is no source: no waffle, no number, `note` says it. */
    usedBp: number | null;
    severity?: Severity;
    /** Said in words instead of a number. */
    note?: string;
    selected?: boolean;
    /** A sparkline between the window and the figures (S48). */
    trace?: Snippet;
    onclick: () => void;
  }

  const {
    name,
    window: windowText,
    usedBp,
    severity = "unknown",
    note = "No source Flashboard can read yet.",
    selected = false,
    trace,
    onclick,
  }: Props = $props();
</script>

<!-- S42: one 40 px row per provider window. -->
<button
  type="button"
  class:selected
  aria-current={selected ? "true" : undefined}
  {onclick}
>
  <span class="name">{name}</span>
  <span class="window">{usedBp === null ? note : (windowText ?? "")}</span>
  {#if trace}{@render trace()}{/if}
  {#if usedBp !== null}
    <Meter {usedBp} {severity} label={name} size="mini" decorative />
    <span
      class="number"
      class:alert={severity === "warn" || severity === "critical"}
      >{percent(usedBp)}</span
    >
  {/if}
</button>

<style>
  button {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    box-sizing: border-box;
    inline-size: 100%;
    min-block-size: calc(var(--space-8) + var(--space-2));
    padding: var(--space-1) var(--space-3);
    text-align: start;
    background: none;
    border: 0;
    border-block-end: var(--rule) solid var(--color-rule);
    border-radius: 0;
  }
  button:hover {
    background: var(--color-hover);
  }
  button.selected {
    box-shadow: inset 2px 0 0 var(--color-ink);
    background: var(--color-selected);
  }
  .name {
    flex: 0 1 10rem;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    font-weight: var(--text-md-weight-strong);
    color: var(--color-ink);
  }
  .window {
    flex: 1 1 auto;
    min-inline-size: 0;
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
  }
  .selected .window {
    color: var(--color-ink);
  }
  .number {
    flex: 0 0 3.5rem;
    margin-inline-start: var(--space-3);
    text-align: end;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-ink);
  }
  .number.alert {
    color: var(--color-alert);
  }
</style>
