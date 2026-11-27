<!-- One attachment as a chip (C1 attachments): a picture's preview or a type tag, the name in
     mono and a second line (size, "84 lines", "Reading", or what's wrong). In the composer it
     has a remove button, shown on hover and focus; a refused chip keeps it showing, since
     removing it is the way to send. -->
<script lang="ts">
  import type { Action } from "svelte/action";

  interface Props {
    name: string;
    meta: string;
    /** The type tag ("CSS", "PDF", "¶" for pasted text) when there's no preview. */
    tag: string;
    status?: "ready" | "reading" | "refused";
    /** A picture's preview, drawn on a canvas. */
    preview?: ImageBitmap | null;
    /** The larger picture of a sent message. */
    large?: boolean;
    onremove?: (() => void) | undefined;
  }

  const {
    name,
    meta,
    tag,
    status = "ready",
    preview = null,
    large = false,
    onremove,
  }: Props = $props();

  /** Draws `bitmap` to fill the canvas (cropped to cover), sharp at the screen's density. */
  const draw: Action<HTMLCanvasElement, ImageBitmap> = (canvas, bitmap) => {
    function paint(image: ImageBitmap): void {
      const ratio = window.devicePixelRatio || 1;
      const w = Math.round(canvas.clientWidth * ratio);
      const h = Math.round(canvas.clientHeight * ratio);
      if (w === 0 || h === 0) return;
      canvas.width = w;
      canvas.height = h;
      const scale = Math.max(w / image.width, h / image.height);
      const sw = w / scale;
      const sh = h / scale;
      canvas
        .getContext("2d")
        ?.drawImage(
          image,
          (image.width - sw) / 2,
          (image.height - sh) / 2,
          sw,
          sh,
          0,
          0,
          w,
          h,
        );
    }
    paint(bitmap);
    return { update: paint };
  };
</script>

{#if large && preview}
  <figure class="sent-image">
    <canvas class="sent-canvas" use:draw={preview} aria-hidden="true"></canvas>
    <figcaption>{name} · {meta}</figcaption>
  </figure>
{:else}
  <span class="chip" data-status={status} class:image={preview !== null}>
    {#if preview}
      <canvas class="thumb" use:draw={preview} aria-hidden="true"></canvas>
    {:else}
      <span class="tag" aria-hidden="true">{tag}</span>
    {/if}
    <span class="text">
      <span class="name">{name}</span>
      <span class="meta">
        {#if status === "reading"}<span class="reading" aria-hidden="true"
            >{#each { length: 10 }, i (i)}<span></span>{/each}</span
          >{/if}
        {meta}
      </span>
    </span>
    {#if onremove}
      <button
        type="button"
        class="remove"
        aria-label={`Remove ${name}`}
        onclick={onremove}
      >
        <svg viewBox="0 0 10 10" aria-hidden="true"
          ><path d="M2 2l6 6M8 2 2 8" /></svg
        >
      </button>
    {/if}
  </span>
{/if}

<style>
  .chip {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    max-width: var(--attach-chip-max);
    height: var(--attach-h);
    padding: 0 var(--space-3) 0 var(--space-2);
    border-radius: var(--radius-dialog);
    background: var(--color-chip);
    animation: chip-in var(--duration-spring-firm) var(--ease-spring-firm) both;
  }
  .chip.image {
    padding-inline-start: 0;
  }
  .chip[data-status="refused"] {
    padding-inline-end: var(--space-6);
    background: var(--color-paper);
    box-shadow: inset 0 0 0 var(--rule) var(--color-alert);
  }
  .tag {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--attach-tag);
    height: var(--attach-tag);
    border-radius: var(--radius-small);
    background: var(--color-paper);
    box-shadow: inset 0 0 0 var(--rule) var(--color-rule);
    color: var(--color-muted);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    letter-spacing: 0.04em;
  }
  [data-status="refused"] .tag {
    box-shadow: inset 0 0 0 var(--rule) var(--color-alert);
    color: var(--color-alert);
  }
  .thumb {
    flex: none;
    width: var(--attach-thumb-w);
    height: var(--attach-h);
    border-radius: var(--radius-dialog) 0 0 var(--radius-dialog);
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-muted);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
    white-space: nowrap;
  }
  [data-status="refused"] .name,
  [data-status="refused"] .meta {
    color: var(--color-alert);
  }
  /* Reading: a row of ten dots filling one by one (the waffle's grain), then again. */
  .reading {
    display: inline-flex;
    gap: calc(var(--attach-dot) / 2);
  }
  .reading span {
    width: var(--attach-dot);
    height: var(--attach-dot);
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 var(--rule) var(--color-faint);
    animation: fill-dot calc(var(--duration-slow) * 6) var(--ease) infinite;
  }
  .reading span:nth-child(2) {
    animation-delay: calc(var(--duration-slow) * 0.4);
  }
  .reading span:nth-child(3) {
    animation-delay: calc(var(--duration-slow) * 0.8);
  }
  .reading span:nth-child(4) {
    animation-delay: calc(var(--duration-slow) * 1.2);
  }
  .reading span:nth-child(5) {
    animation-delay: calc(var(--duration-slow) * 1.6);
  }
  .reading span:nth-child(6) {
    animation-delay: calc(var(--duration-slow) * 2);
  }
  .reading span:nth-child(7) {
    animation-delay: calc(var(--duration-slow) * 2.4);
  }
  .reading span:nth-child(8) {
    animation-delay: calc(var(--duration-slow) * 2.8);
  }
  .reading span:nth-child(9) {
    animation-delay: calc(var(--duration-slow) * 3.2);
  }
  .reading span:nth-child(10) {
    animation-delay: calc(var(--duration-slow) * 3.6);
  }
  .remove {
    position: absolute;
    inset-block-start: calc(var(--space-2) * -0.75);
    inset-inline-end: calc(var(--space-2) * -0.75);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--space-6);
    min-width: 0;
    height: var(--space-6);
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: var(--radius-full);
    background: var(--color-ink);
    color: var(--color-paper);
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease);
  }
  .remove svg {
    width: var(--space-2);
    height: var(--space-2);
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: 1.4;
  }
  .chip:hover .remove,
  .remove:focus-visible,
  [data-status="refused"] .remove {
    opacity: 1;
  }
  .remove:hover:not(:disabled) {
    background: var(--color-ink-hover);
  }
  .sent-image {
    display: inline-flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
  }
  .sent-canvas {
    width: var(--attach-sent-w);
    height: var(--attach-sent-h);
    border-radius: var(--radius-control);
    box-shadow: inset 0 0 0 var(--rule) var(--color-rule);
  }
  figcaption {
    color: var(--color-muted);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  @keyframes chip-in {
    from {
      opacity: 0;
      scale: 0.92;
      translate: 0 var(--space-1);
    }
    to {
      opacity: 1;
      scale: 1;
      translate: 0 0;
    }
  }
  @keyframes fill-dot {
    0%,
    8% {
      background: transparent;
    }
    20%,
    80% {
      background: var(--color-ink);
    }
    100% {
      background: transparent;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .chip,
    .reading span {
      animation: none;
    }
  }
</style>
