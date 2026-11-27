<script lang="ts">
  import { announce } from "../../stores/announce.svelte";
</script>

<div class="live-region" aria-live="polite" aria-atomic="true">
  {announce.current}
</div>

<style>
  .live-region {
    /* Pinned to the top so it never adds a pixel of scroll under a full-height view. */
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    width: 1px;
    height: 1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
