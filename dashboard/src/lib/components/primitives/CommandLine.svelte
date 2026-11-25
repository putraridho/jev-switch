<script lang="ts">
  import CopyButton from "./CopyButton.svelte";

  interface Props {
    /** The command, or the variable name, to show and copy. */
    command: string;
  }

  const { command }: Props = $props();
</script>

<div class="line">
  <code>{command}</code>
  <CopyButton text={command} />
</div>

<style>
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  code {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--color-chip);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-control);
    overflow-wrap: anywhere;
  }
</style>
