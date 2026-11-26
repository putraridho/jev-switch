<!-- The header strip (S25): wordmark and the warning-mark legend on the left, freshness and
     the view's controls on the right. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import WarningLegend from "./WarningLegend.svelte";

  interface Props {
    warnAtPct?: number;
    /** Replaces the plain wordmark, for example with a link home. */
    wordmark?: Snippet;
    /** Freshness and controls. */
    children?: Snippet;
  }

  const { warnAtPct = 80, wordmark, children }: Props = $props();
</script>

<header>
  <div class="group">
    {#if wordmark}{@render wordmark()}{:else}<span class="wordmark"
        >Flashboard</span
      >{/if}
    <WarningLegend {warnAtPct} />
  </div>
  {#if children}<div class="group">{@render children()}</div>{/if}
</header>

<style>
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    min-height: var(--header-h);
  }

  .group {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-8);
    min-width: 0;
  }

  .wordmark {
    font-size: var(--text-wordmark);
    font-weight: var(--text-wordmark-weight);
    line-height: var(--text-md-line);
  }

  @media (max-width: 50rem) {
    .group {
      gap: var(--space-4);
    }
  }
</style>
