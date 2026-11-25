<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    /** One line: "Nothing to watch yet." */
    title: string;
    body: string;
    /** A row of faint outlines: only for a connected source with no usage yet. */
    ghost?: boolean;
    /** The one action. */
    action?: Snippet;
  }

  const { title, body, ghost = false, action }: Props = $props();
</script>

<div class="empty">
  {#if ghost}
    <svg viewBox="0 0 240 24" width="240" height="24" aria-hidden="true">
      {#each Array.from({ length: 10 }, (_, i) => i * 24 + 12) as x (x)}
        <circle cx={x} cy="12" r="7.25" />
      {/each}
    </svg>
  {/if}
  <p class="title">{title}</p>
  <p class="body">{body}</p>
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>

<style>
  .empty {
    display: grid;
    justify-items: start;
    gap: var(--space-1);
  }
  svg {
    max-inline-size: 100%;
    margin-block-end: var(--space-3);
    margin-inline-start: calc(var(--space-1) * -1);
  }
  circle {
    fill: none;
    stroke: var(--color-rule);
    stroke-width: 1.5;
  }
  p {
    margin: 0;
  }
  .title {
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    font-weight: var(--text-md-weight-strong);
    color: var(--color-ink);
  }
  .body {
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
  }
  .action {
    margin-block-start: var(--space-3);
  }
</style>
