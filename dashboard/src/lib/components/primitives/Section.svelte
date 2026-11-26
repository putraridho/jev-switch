<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    /** The section label, 14/600, below the rule. */
    label?: string;
    /** The secondary section of a page: the biggest gap (72) above its rule. */
    secondary?: boolean;
    children: Snippet;
  }

  const { label, secondary = false, children }: Props = $props();
</script>

<!-- S31, S44: one hairline above the label, 32 either side, 16 to the content. -->
<section class:secondary>
  {#if label !== undefined}<h2>{label}</h2>{/if}
  {@render children()}
</section>

<style>
  section {
    margin-block-start: var(--space-8);
    padding-block-start: var(--space-8);
    border-block-start: var(--rule) solid var(--color-rule);
  }
  section.secondary {
    margin-block-start: var(--space-18);
  }
  h2 {
    margin: 0 0 var(--space-4);
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    font-weight: var(--text-md-weight-strong);
    color: var(--color-ink);
  }
</style>
