<!-- A small waffle of one plan window with a turn's share marked (C1): the dots the turn
     used pop in, one after another, in the critical fill. Decorative: the sentence beside it
     says the same figures. -->
<script lang="ts">
  interface Props {
    /** Usage before the turn, basis points. */
    beforeBp: number;
    /** Usage after it, basis points. */
    afterBp: number;
  }

  const { beforeBp, afterBp }: Props = $props();

  const before = $derived(
    Math.max(0, Math.min(100, Math.floor(beforeBp / 100))),
  );
  const after = $derived(
    Math.max(before, Math.min(100, Math.floor(afterBp / 100))),
  );
  const dots = $derived(
    Array.from({ length: 100 }, (_, i) => ({
      x: (i % 10) * 24 + 12,
      y: Math.floor(i / 10) * 24 + 12,
      kind: i < before ? "used" : i < after ? "spent" : "free",
      // The order the spent dots pop in; past the eighth they arrive together.
      order: Math.min(8, Math.max(0, i - before)),
    })),
  );
</script>

<svg class="spend" viewBox="0 0 240 240" aria-hidden="true">
  {#each dots as dot, i (i)}
    <circle
      class={dot.kind}
      data-order={dot.kind === "spent" ? dot.order : undefined}
      cx={dot.x}
      cy={dot.y}
      r={dot.kind === "free" ? 6 : 8}
    />
  {/each}
</svg>

<style>
  .spend {
    flex: none;
    width: var(--space-12);
    height: var(--space-12);
  }
  .used {
    fill: var(--waffle-fill);
  }
  .spent {
    fill: var(--color-critical-fill);
    transform-box: fill-box;
    transform-origin: center;
    animation: pop var(--duration-spring-pop) var(--ease-spring-pop) both;
  }
  .free {
    fill: none;
    stroke: var(--color-rule);
    stroke-width: 2;
  }
  .spent[data-order="1"] {
    animation-delay: calc(var(--duration-stagger) * 4);
  }
  .spent[data-order="2"] {
    animation-delay: calc(var(--duration-stagger) * 8);
  }
  .spent[data-order="3"] {
    animation-delay: calc(var(--duration-stagger) * 12);
  }
  .spent[data-order="4"] {
    animation-delay: calc(var(--duration-stagger) * 16);
  }
  .spent[data-order="5"] {
    animation-delay: calc(var(--duration-stagger) * 20);
  }
  .spent[data-order="6"] {
    animation-delay: calc(var(--duration-stagger) * 24);
  }
  .spent[data-order="7"] {
    animation-delay: calc(var(--duration-stagger) * 28);
  }
  .spent[data-order="8"] {
    animation-delay: calc(var(--duration-stagger) * 32);
  }

  @keyframes pop {
    from {
      scale: 0.2;
    }
    to {
      scale: 1;
    }
  }
  @media (forced-colors: active) {
    .used,
    .spent {
      fill: currentColor;
    }
    .free {
      stroke: currentColor;
    }
  }
</style>
