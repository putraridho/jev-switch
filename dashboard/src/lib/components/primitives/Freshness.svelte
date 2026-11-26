<script lang="ts">
  import { clock } from "../../stores/clock.svelte";
  import { freshness } from "./display";
  import StatusBadge from "./StatusBadge.svelte";

  interface Props {
    /** When the reading was taken (epoch milliseconds). */
    atMs: number;
  }

  const { atMs }: Props = $props();
  const age = $derived(freshness(atMs, clock.now));
</script>

{#if age.stale}
  <StatusBadge kind="stale" label={age.text} />
{:else}
  <span>{age.text}</span>
{/if}

<style>
  span {
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
