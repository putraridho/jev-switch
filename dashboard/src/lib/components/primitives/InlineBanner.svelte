<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    body: string;
    /** The outline Retry button. */
    action?: Snippet;
  }

  const { title, body, action }: Props = $props();
</script>

<!-- S40: sits above the provider it describes and stays until the source answers. A failed
     source is not a threshold, so it stays ink. -->
<div class="banner">
  <div class="words">
    <p class="title">{title}</p>
    <p class="body">{body}</p>
  </div>
  {#if action}{@render action()}{/if}
</div>

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding-block: var(--space-3);
    border-block: var(--rule) solid var(--color-faint);
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
</style>
