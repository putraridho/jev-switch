<script lang="ts">
  interface Props {
    size?: "large" | "medium";
    /** Heard once by a screen reader; give the others in a group an empty string. */
    announcement?: string;
  }

  const { size = "large", announcement = "Loading usage" }: Props = $props();

  // Ten diagonal bands, top left to bottom right (S46).
  const dots = Array.from({ length: 100 }, (_, i) => ({
    x: (i % 10) * 24 + 12,
    y: Math.floor(i / 10) * 24 + 12,
    band: Math.floor(((i % 10) + Math.floor(i / 10)) / 2),
  }));
</script>

<div role="status">
  <svg
    viewBox="0 0 256 240"
    width={size === "large" ? 256 : 128}
    height={size === "large" ? 240 : 120}
    aria-hidden="true"
  >
    {#each dots as dot, i (i)}
      <circle class="b{dot.band}" cx={dot.x} cy={dot.y} r="7.25" />
    {/each}
  </svg>
  <span class="visually-hidden">{announcement}</span>
</div>

<style>
  svg {
    display: block;
    margin-inline-start: calc(var(--space-1) * -1);
  }
  circle {
    fill: none;
    stroke: var(--waffle-empty);
    stroke-width: 1.5;
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  /* The quiet pulse: staged, so keyframes; still without motion permission. */
  @media (prefers-reduced-motion: no-preference) {
    circle {
      animation: pulse 1600ms var(--ease) infinite;
    }
    .b1 {
      animation-delay: 80ms;
    }
    .b2 {
      animation-delay: 160ms;
    }
    .b3 {
      animation-delay: 240ms;
    }
    .b4 {
      animation-delay: 320ms;
    }
    .b5 {
      animation-delay: 400ms;
    }
    .b6 {
      animation-delay: 480ms;
    }
    .b7 {
      animation-delay: 560ms;
    }
    .b8 {
      animation-delay: 640ms;
    }
    .b9 {
      animation-delay: 720ms;
    }
  }
  @keyframes pulse {
    0%,
    40%,
    100% {
      opacity: 1;
    }
    20% {
      opacity: 0.3;
    }
  }
</style>
