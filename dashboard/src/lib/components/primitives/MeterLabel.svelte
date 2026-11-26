<script lang="ts">
  import type { StatusKind } from "./display";
  import StatusBadge from "./StatusBadge.svelte";

  interface Props {
    /** The provider (14/600); omit it under a provider heading. */
    name?: string | undefined;
    /** The window, or "balance". Never shrinks. */
    window: string;
    /** Pushed to the right of the line. */
    status?: StatusKind | undefined;
  }

  const { name, window: windowText, status }: Props = $props();
</script>

<p class="label">
  {#if name !== undefined}<span class="name">{name}</span>{/if}
  <span class="window" class:alone={name === undefined}>{windowText}</span>
  {#if status !== undefined}
    <span class="status"><StatusBadge kind={status} /></span>
  {/if}
</p>

<style>
  .label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--text-md-line);
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    white-space: nowrap;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: var(--text-md-weight-strong);
    color: var(--color-ink);
  }
  .window {
    flex-shrink: 0;
    color: var(--color-muted);
  }
  .window.alone {
    color: var(--color-ink);
    font-weight: var(--text-md-weight-strong);
  }
  .status {
    flex-shrink: 0;
    margin-inline-start: auto;
  }
</style>
