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
  /* One line with an ellipsis, the button full size (S29). Below 30 rem (so at 320 px and
     at 200 % zoom) the chip wraps instead: D9.10 allows no clipped text. */
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    max-width: 100%;
  }

  code {
    min-width: 0;
    padding-inline: var(--space-3);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: var(--control-h);
    background: var(--color-chip);
    border-radius: var(--radius-control);
  }

  @media (max-width: 30rem) {
    code {
      white-space: normal;
      overflow-wrap: anywhere;
      line-height: var(--text-sm-line);
      padding-block: var(--space-2);
    }
  }
</style>
