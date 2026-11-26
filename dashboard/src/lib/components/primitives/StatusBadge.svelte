<script lang="ts">
  import type { StatusKind } from "./display";

  interface Props {
    kind: StatusKind;
    /** Replaces the kind's default word. */
    label?: string;
  }

  const { kind, label }: Props = $props();

  const words: Record<StatusKind, string> = {
    near_limit: "Near limit",
    stale: "Stale",
    healthy: "Healthy",
    no_source: "No source",
  };
</script>

<span class="badge {kind}">
  <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
    {#if kind === "near_limit"}
      <path d="M8 1.75 L15 14 L1 14 Z M8 6.5 V9.75 M8 11.75 h0.01" />
    {:else if kind === "stale"}
      <circle cx="8" cy="8" r="6.25" />
      <path d="M8 4.5 V8 L10.25 9.5" />
    {:else if kind === "healthy"}
      <circle cx="8" cy="8" r="6.25" />
      <path d="M5.25 8.25 L7.25 10.25 L11 6.25" />
    {:else}
      <circle cx="8" cy="8" r="6.25" />
      <path d="M5.25 8 H10.75" />
    {/if}
  </svg>
  <span>{label ?? words[kind]}</span>
</span>

<style>
  .badge {
    --badge-gap: var(--space-2);

    display: inline-flex;
    align-items: center;
    gap: var(--badge-gap);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    font-weight: var(--text-sm-weight-strong);
    min-width: 0;
  }
  svg {
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .near_limit {
    color: var(--color-alert);
  }
  .stale {
    color: var(--color-stale);
  }
  .healthy {
    color: var(--color-ink);
  }
  .no_source {
    color: var(--color-muted);
  }
</style>
