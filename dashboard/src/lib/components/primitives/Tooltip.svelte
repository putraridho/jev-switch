<!-- The tooltip (S37): ink on paper reversed, 12/16, 8 px from its target, opens after
     300 ms on hover or focus, flips below when there is no room above. `Esc` and scroll
     close it. It stays while the pointer is over it (WCAG 1.4.13). -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { pushEscape } from "../../escape";

  interface Props {
    text: string;
    /** The target. Put the given id in its `aria-describedby`. */
    children: Snippet<[string]>;
    delayMs?: number;
  }

  const { text, children, delayMs = 300 }: Props = $props();

  const uid = $props.id();
  /** The tooltip's height plus its gap: less room than this above, and it flips. */
  const NEEDED_PX = 36;
  /** The tooltip keeps this far from the frame's side edges. */
  const EDGE_PX = 8;

  let open = $state(false);
  let below = $state(false);
  let align = $state<"center" | "start" | "end">("center");
  let root = $state<HTMLElement | null>(null);
  let tip = $state<HTMLElement | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function cancel(): void {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  function show(): void {
    cancel();
    timer = setTimeout(() => {
      const rect = root?.getBoundingClientRect();
      below = (rect?.top ?? NEEDED_PX) < NEEDED_PX;
      const half = (tip?.offsetWidth ?? 0) / 2;
      const middle = rect === undefined ? half : rect.left + rect.width / 2;
      const frame = document.documentElement.clientWidth;
      if (middle - half < EDGE_PX) align = "start";
      else if (middle + half > frame - EDGE_PX) align = "end";
      else align = "center";
      open = true;
    }, delayMs);
  }

  function hide(): void {
    cancel();
    open = false;
  }

  $effect(() => {
    if (!open) return;
    window.addEventListener("scroll", hide, true);
    const pop = pushEscape("disclosure", () => {
      hide();
      return true;
    });
    return () => {
      window.removeEventListener("scroll", hide, true);
      pop();
    };
  });

  $effect(() => cancel);
</script>

<span
  class="anchor"
  role="presentation"
  bind:this={root}
  onpointerenter={show}
  onpointerleave={hide}
  onfocusin={show}
  onfocusout={hide}
>
  {@render children(uid)}
  <span
    class="tip"
    class:open
    class:below
    class:start={align === "start"}
    class:end={align === "end"}
    role="tooltip"
    id={uid}
    bind:this={tip}>{text}</span
  >
</span>

<style>
  .anchor {
    position: relative;
    display: inline-flex;
  }

  .tip {
    position: absolute;
    bottom: calc(100% + var(--tooltip-gap));
    left: 50%;
    z-index: 2;
    translate: -50% 0;
    width: max-content;
    max-width: min(var(--menu-w), calc(100vw - 2 * var(--space-2)));
    padding: calc(var(--space-2) - var(--segment-inset)) var(--space-3);
    color: var(--color-paper);
    background: var(--color-ink);
    border-radius: var(--radius-control);
    font-size: var(--text-micro);
    font-weight: var(--text-micro-weight);
    line-height: var(--text-micro-line);
    visibility: hidden;
  }

  .start {
    left: 0;
    translate: 0 0;
  }

  .end {
    right: 0;
    left: auto;
    translate: 0 0;
  }

  /* A bridge over the gap, so the pointer can reach the tooltip. */
  .tip::after {
    content: "";
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    height: var(--tooltip-gap);
  }

  .below {
    top: calc(100% + var(--tooltip-gap));
    bottom: auto;
  }

  .below::after {
    top: auto;
    bottom: 100%;
  }

  .open {
    visibility: visible;
  }

  @media (forced-colors: active) {
    .tip {
      border: var(--outline) solid currentColor;
    }
  }
</style>
