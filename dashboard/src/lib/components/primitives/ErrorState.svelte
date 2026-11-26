<script lang="ts">
  import type { Snippet } from "svelte";
  import StaleRow from "./StaleRow.svelte";

  interface Props {
    /** "Stale. Last read 2 hours ago." Omit when nothing was ever read. */
    staleText?: string;
    /** One sentence: what happened. End it with "Run:" when a command follows. */
    message: string;
    /** The command chip with its copy button. */
    command?: Snippet;
    /** Retry, or another single action. */
    action?: Snippet;
  }

  const { staleText, message, command, action }: Props = $props();
</script>

<!-- S28: no number and no waffle while the source cannot be read. Never alert colour. -->
<div class="error-state">
  {#if staleText !== undefined}<StaleRow text={staleText} />{/if}
  <p>{message}</p>
  {#if command}<div class="command">{@render command()}</div>{/if}
  {#if action}<div>{@render action()}</div>{/if}
</div>

<style>
  .error-state {
    display: grid;
    justify-items: start;
    gap: var(--space-2);
    max-inline-size: 100%;
  }
  p {
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    color: var(--color-ink);
  }
  .command {
    max-inline-size: 100%;
  }
</style>
