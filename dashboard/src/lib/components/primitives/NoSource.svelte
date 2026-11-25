<script lang="ts">
  interface Props {
    /** The providers without a usage source, all in one block. */
    names: readonly string[];
  }

  const { names }: Props = $props();
</script>

<!-- S30: no waffle, no zero, no dash: an empty meter would claim a measurement. -->
{#if names.length > 0}
  <div class="no-source">
    <p class="names">{names.join(", ")}</p>
    <p>No source Flashboard can read yet.</p>
    <p>Flashboard shows no number rather than a guess.</p>
  </div>
{/if}

<style>
  .no-source {
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
  }
  p {
    margin: 0;
  }
  .names {
    margin-block-end: var(--space-1);
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    font-weight: var(--text-md-weight-strong);
  }
</style>
