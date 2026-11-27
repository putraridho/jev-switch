<!-- The app's header strip (S25, S50): wordmark and warning-mark legend, then how old the
     newest reading is, Refresh on the overview, and the two views. -->
<script lang="ts">
  import { refreshAll } from "../../actions";
  import { router } from "../../router.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { usage } from "../../stores/usage.svelte";
  import AppHeader from "../primitives/AppHeader.svelte";
  import Freshness from "../primitives/Freshness.svelte";
  import IconButton from "../primitives/IconButton.svelte";

  const views = [
    { name: "overview", label: "Usage" },
    { name: "chat", label: "Chat" },
    { name: "settings", label: "Settings" },
  ] as const;

  function open(name: "overview" | "chat" | "settings"): void {
    if (router.route.name !== name) router.go({ name });
  }

  const newest = $derived.by(() => {
    let at: number | null = null;
    for (const item of usage.snapshot?.providers ?? []) {
      if (item.usage.state !== "known" && item.usage.state !== "stale")
        continue;
      at = Math.max(at ?? 0, item.usage.reading.observed_at);
    }
    return at;
  });
</script>

<div class="bar">
  <div class="inner">
    <AppHeader warnAtPct={Math.round(settings.value.warn_bp / 100)}>
      {#if newest !== null}<Freshness atMs={newest} />{/if}
      <div class="controls">
        {#if router.route.name === "overview"}
          <IconButton
            label="Refresh"
            icon="refresh"
            size="small"
            quiet
            disabled={usage.refreshing}
            onclick={refreshAll}
          />
        {/if}
        <nav aria-label="Flashboard">
          {#each views as view (view.name)}
            <button
              type="button"
              class="pill"
              aria-current={router.route.name === view.name
                ? "page"
                : undefined}
              onclick={() => {
                open(view.name);
              }}>{view.label}</button
            >
          {/each}
        </nav>
      </div>
    </AppHeader>
  </div>
</div>

<style>
  .bar {
    border-bottom: var(--rule) solid var(--color-rule);
    background: var(--color-bg);
    color: var(--color-text);
    font-family: var(--font-sans);
  }
  .inner {
    box-sizing: border-box;
    max-width: 90rem;
    margin-inline: auto;
    padding: var(--space-2) var(--space-4);
  }
  .controls,
  nav {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .controls {
    gap: var(--space-2);
  }
  .pill {
    padding-inline: var(--space-3);
    border-color: transparent;
    border-radius: var(--radius-pill);
    color: var(--color-muted);
    font-size: var(--text-sm);
    font-weight: var(--text-sm-weight-strong);
  }
  .pill:hover:not(:disabled):not([aria-current="page"]) {
    color: var(--color-ink);
  }
  /* The open view is marked by a filled pill and aria-current, not by colour alone. */
  .pill[aria-current="page"] {
    background: var(--color-ink);
    color: var(--color-bg);
  }

  @media (min-width: 40rem) {
    .inner {
      padding-inline: var(--space-6);
    }
  }
  @media (min-width: 80rem) {
    .inner {
      padding-inline: var(--space-18);
    }
  }

  @media (forced-colors: active) {
    .pill[aria-current="page"] {
      background: transparent;
      color: inherit;
      text-decoration: underline;
    }
  }
</style>
