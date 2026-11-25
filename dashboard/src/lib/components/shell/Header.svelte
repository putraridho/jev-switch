<script lang="ts">
  import { router } from "../../router.svelte";

  function open(name: "overview" | "settings"): void {
    if (router.route.name !== name) router.go({ name });
  }
</script>

<header class="app-header">
  <nav aria-label="Flashboard">
    <button
      type="button"
      class="brand"
      aria-current={router.route.name === "overview" ? "page" : undefined}
      onclick={() => {
        open("overview");
      }}>Flashboard</button
    >
    <button
      type="button"
      aria-current={router.route.name === "settings" ? "page" : undefined}
      onclick={() => {
        open("settings");
      }}>Settings</button
    >
  </nav>
</header>

<style>
  .app-header {
    border-bottom: var(--rule) solid var(--color-rule);
  }

  nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    min-height: var(--space-12);
    max-width: 40rem;
    margin-inline: auto;
    padding-inline: var(--space-4);
  }

  .brand {
    border: none;
    padding-inline: 0;
    font-weight: var(--text-md-weight-strong);
  }

  [aria-current="page"] {
    color: var(--color-text);
    font-weight: var(--text-md-weight-strong);
  }
</style>
