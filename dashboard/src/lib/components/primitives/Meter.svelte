<script lang="ts">
  import { pushEscape } from "../../escape";
  import type { Severity } from "../../ipc/bindings/Severity";
  import type { WaffleSize } from "./display";

  interface Props {
    /** Usage in basis points, 0..=10000. */
    usedBp: number;
    /** The level: drawn as fill colour past the warning mark. */
    severity: Severity;
    /** The accessible name, required. */
    label: string;
    /** The full level-1 sentence for `aria-valuetext`; "N% used" when absent. */
    valueText?: string;
    /** Makes the meter a tab stop, so keyboard users reach its detail. */
    focusable?: boolean;
    size?: WaffleSize;
    /** The last good value after a failed read: outlines only, no alert colour (S27). */
    stale?: boolean;
    /** Where the warning triangle sits, in percent (S17). */
    warnAtPct?: number;
    /** The critical triangle shows once usage reaches this percent (S17). */
    criticalAtPct?: number;
    /** A picture beside text that already says the figure: no meter role. */
    decorative?: boolean;
    /** Names a dot on hover or focus, "Dot N of 100" (S37); large and medium only. */
    dotTooltip?: boolean;
  }

  const {
    usedBp,
    severity,
    label,
    valueText,
    focusable = false,
    size = "large",
    stale = false,
    warnAtPct = 80,
    criticalAtPct = 95,
    decorative = false,
    dotTooltip = false,
  }: Props = $props();

  // The waffle is a 10x10 grid: 100 cells, row-major from the top-left. Below the warning
  // mark the fill is ink; cells past it take the one alert colour once the level is raised.
  const valueNow = $derived(
    Math.min(100, Math.max(0, Math.round(usedBp / 100))),
  );
  const raised = $derived(
    !stale && (severity === "warn" || severity === "critical"),
  );
  const mini = $derived(size === "mini");
  const dots = $derived(
    Array.from({ length: 100 }, (_, i) => {
      const filled = i < valueNow;
      if (mini) {
        const fill = !filled
          ? "var(--color-rule)"
          : i >= warnAtPct && raised
            ? "var(--color-critical-fill)"
            : "var(--waffle-fill)";
        return { r: filled ? 10 : 7, fill, stroke: "none" };
      }
      if (filled && !stale) {
        const fill =
          i >= warnAtPct && raised
            ? "var(--color-critical-fill)"
            : "var(--waffle-fill)";
        return { r: 8, fill, stroke: "none" };
      }
      return {
        r: 7.25,
        fill: "none",
        stroke: stale && !filled ? "var(--color-rule)" : "var(--waffle-empty)",
      };
    }),
  );

  // The triangle sits just outside the grid, at the row that holds the mark's cell.
  function markerY(pct: number): number {
    const cell = Math.min(100, Math.max(1, Math.round(pct)));
    return Math.floor((cell - 1) / 10) * 24 + 12;
  }
  function triangle(y: number): string {
    return `M242 ${String(y)} L252 ${String(y - 6)} L252 ${String(y + 6)} Z`;
  }
  const criticalY = $derived(
    !stale && valueNow >= criticalAtPct ? markerY(criticalAtPct) : null,
  );
  const warnY = $derived(
    markerY(warnAtPct) === criticalY ? null : markerY(warnAtPct),
  );

  const px = {
    large: { width: 256, height: 240 },
    medium: { width: 128, height: 120 },
    small: { width: 64, height: 60 },
    mini: { width: 24, height: 24 },
  } as const;
  // The dot tooltip (S37): 300 ms to open, instant between dots, closed at once by
  // leaving, `Esc` or scroll. It is a picture of what `aria-valuetext` already says.
  const TIP_DELAY_MS = 300;
  /** Tooltip size and gap in screen pixels, and the room it needs above its dot. */
  const TIP = { width: 100, height: 28, gap: 8, radius: 6 } as const;
  const named = $derived(
    dotTooltip && !decorative && (size === "large" || size === "medium"),
  );
  /** Grid units per screen pixel: the medium waffle is drawn at half size. */
  const unit = $derived(size === "medium" ? 2 : 1);
  let tip = $state<{ dot: number; below: boolean } | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function cancelTip(): void {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  function hideTip(): void {
    cancelTip();
    tip = null;
  }

  function showTip(dot: number, target: Element | null): void {
    cancelTip();
    const open = (): void => {
      const top = target?.getBoundingClientRect().top;
      tip = { dot, below: top !== undefined && top < TIP.height + TIP.gap };
    };
    if (tip === null) timer = setTimeout(open, TIP_DELAY_MS);
    else open();
  }

  function overDot(event: PointerEvent): void {
    if (!(event.target instanceof Element)) return;
    const dot = event.target.getAttribute("data-dot");
    if (dot !== null) showTip(Number(dot), event.target);
  }

  /** Keyboard focus names the last filled dot, the one that carries the value. */
  function focusDot(event: FocusEvent): void {
    if (!named || valueNow === 0) return;
    const holder =
      event.currentTarget instanceof Element ? event.currentTarget : null;
    showTip(
      valueNow - 1,
      holder?.querySelector(`[data-dot="${String(valueNow - 1)}"]`) ?? null,
    );
  }

  const bubble = $derived.by(() => {
    if (tip === null) return null;
    const cx = (tip.dot % 10) * 24 + 12;
    const cy = Math.floor(tip.dot / 10) * 24 + 12;
    const width = TIP.width * unit;
    const reach = (TIP.height + TIP.gap) * unit + 8;
    return {
      label: `Dot ${String(tip.dot + 1)} of 100`,
      x: Math.min(256 - width, Math.max(0, cx - width / 2)),
      y: tip.below ? cy + 8 + TIP.gap * unit : cy - reach,
      /** The invisible strip over the gap, so the pointer can reach the tooltip. */
      bridge: tip.below ? -TIP.gap : TIP.height,
    };
  });

  $effect(() => {
    if (tip === null) return;
    window.addEventListener("scroll", hideTip, true);
    const pop = pushEscape("disclosure", () => {
      hideTip();
      return true;
    });
    return () => {
      window.removeEventListener("scroll", hideTip, true);
      pop();
    };
  });

  $effect(() => cancelTip);

  const text = $derived(
    valueText ??
      (severity === "unknown" ? "No data" : `${String(valueNow)}% used`),
  );
</script>

{#snippet waffle()}
  <svg
    class="waffle {size}"
    viewBox={mini ? "0 0 240 240" : "0 0 256 240"}
    width={px[size].width}
    height={px[size].height}
    aria-hidden="true"
    role="presentation"
    onpointerover={named ? overDot : undefined}
    onpointerleave={named ? hideTip : undefined}
  >
    {#each dots as dot, i (i)}
      <circle
        data-dot={named ? i : undefined}
        pointer-events={named ? "all" : undefined}
        cx={(i % 10) * 24 + 12}
        cy={Math.floor(i / 10) * 24 + 12}
        r={dot.r}
        fill={dot.fill}
        stroke={dot.stroke}
        stroke-width={dot.stroke === "none" ? "0" : "1.5"}
      />
    {/each}
    {#if !mini}
      {#if warnY !== null}
        <path class="warn-mark" d={triangle(warnY)} />
      {/if}
      {#if criticalY !== null}
        <path class="critical-mark" d={triangle(criticalY)} />
      {/if}
    {/if}
    {#if bubble !== null}
      <g
        class="dot-tip"
        transform="translate({bubble.x} {bubble.y}) scale({unit})"
      >
        <rect
          class="bridge"
          y={bubble.bridge}
          width={TIP.width}
          height={TIP.gap}
        />
        <rect
          class="bubble"
          width={TIP.width}
          height={TIP.height}
          rx={TIP.radius}
        />
        <text x={TIP.width / 2} y={TIP.height / 2}>{bubble.label}</text>
      </g>
    {/if}
  </svg>
{/snippet}

{#if decorative}
  <span class="meter">{@render waffle()}</span>
{:else}
  <!-- A meter is a tab stop so keyboard users reach its detail (DESIGN 9.3, 9.10). -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="meter"
    role="meter"
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={valueNow}
    aria-valuetext={text}
    tabindex={focusable ? 0 : undefined}
    aria-label={label}
    onfocus={focusDot}
    onblur={hideTip}
  >
    {@render waffle()}
  </div>
{/if}

<style>
  .meter {
    display: inline-block;
    line-height: 0;
  }

  /* Optical alignment: the first dot's edge sits on the text edge, not its cell's. */
  .waffle {
    display: block;
    overflow: visible;
  }
  .bridge {
    fill: transparent;
  }
  .bubble {
    fill: var(--color-ink);
  }
  .dot-tip text {
    fill: var(--color-paper);
    font-size: var(--text-micro);
    text-anchor: middle;
    dominant-baseline: central;
  }
  .large {
    margin-inline-start: calc(var(--space-1) * -1);
  }
  .medium {
    margin-inline-start: calc(var(--space-1) / -2);
  }

  .warn-mark {
    fill: var(--color-muted);
  }
  .critical-mark {
    fill: none;
    stroke: var(--color-alert);
    stroke-width: 1.5;
    stroke-linejoin: round;
  }

  @media (forced-colors: active) {
    /* The tooltip keeps its own ink-on-paper pair, which already contrasts. */
    .dot-tip {
      forced-color-adjust: none;
    }
    circle[fill="none"] {
      stroke: currentColor;
    }
    circle:not([fill="none"]),
    .warn-mark {
      fill: currentColor;
    }
    .critical-mark {
      stroke: currentColor;
    }
  }
</style>
