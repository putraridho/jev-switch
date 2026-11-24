<script lang="ts">
  import type { Severity } from "../../ipc/bindings/Severity";

  interface Props {
    /** Usage in basis points, 0..=10000. */
    usedBp: number;
    /** The level: drawn as fill colour and the warning marker. */
    severity: Severity;
    /** The accessible name, required. */
    label: string;
  }

  const { usedBp, severity, label }: Props = $props();

  // The waffle is a 10x10 grid: 100 cells, row-major from the top-left. Cells past the
  // 80% warning mark fill with the critical colour; below it, ink.
  const valueNow = $derived(
    Math.min(100, Math.max(0, Math.round(usedBp / 100))),
  );
  const critical = $derived(severity === "warn" || severity === "critical");
  const dots = $derived(
    Array.from({ length: 100 }, (_, i) => ({
      filled: i < valueNow,
      fill:
        i >= 80 && critical
          ? "var(--color-critical-fill)"
          : "var(--waffle-fill)",
    })),
  );
  const valueText = $derived(
    severity === "unknown" ? "No data" : `${String(valueNow)}% used`,
  );
</script>

<div
  class="meter"
  role="meter"
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={valueNow}
  aria-valuetext={valueText}
  aria-label={label}
>
  <svg
    class="waffle"
    viewBox="0 0 240 240"
    width="240"
    height="240"
    aria-hidden="true"
  >
    {#each dots as dot, i (i)}
      <circle
        cx={(i % 10) * 24 + 12}
        cy={Math.floor(i / 10) * 24 + 12}
        r={dot.filled ? 8 : 7.25}
        fill={dot.filled ? dot.fill : "none"}
        stroke={dot.filled ? "none" : "var(--waffle-empty)"}
        stroke-width={dot.filled ? "0" : "1.5"}
      />
    {/each}
    {#if critical}
      <path d="M 194 178 L 200 173 L 200 183 Z" fill="var(--color-muted)" />
    {/if}
  </svg>
</div>

<style>
  .meter {
    display: inline-block;
    line-height: 0;
  }

  .waffle {
    display: block;
  }
</style>
