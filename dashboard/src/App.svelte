<script lang="ts">
  import { router } from "./lib/router.svelte";
  import Overview from "./screens/Overview.svelte";
  import ProviderDetail from "./screens/ProviderDetail.svelte";
  import Settings from "./screens/Settings.svelte";
</script>

<main>
  {#if router.route.name === "overview"}
    <Overview />
  {:else if router.route.name === "provider"}
    <ProviderDetail />
  {:else}
    <Settings />
  {/if}
</main>

<style>
  main {
    min-height: 100vh;
    background: var(--color-bg);
  }
</style>
