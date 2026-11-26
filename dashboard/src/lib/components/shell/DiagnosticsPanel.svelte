<script lang="ts">
  import {
    bytesLabel,
    countLabel,
    realSources,
    sourceName,
    sourceStatus,
  } from "../../diagnostics";
  import type { Diagnostics } from "../../ipc/bindings/Diagnostics";
  import { queries } from "../../stores/queries";
  import { settings } from "../../stores/settings.svelte";

  type Loaded =
    | { state: "loading" }
    | { state: "error" }
    | { state: "ready"; value: Diagnostics };
  let loaded = $state<Loaded>({ state: "loading" });

  async function load(): Promise<void> {
    loaded = { state: "loading" };
    const result = await queries.diagnostics();
    loaded =
      result.status === "ok"
        ? { state: "ready", value: result.value }
        : { state: "error" };
  }

  $effect(() => {
    void load();
  });
</script>

<details>
  <summary
    >About and diagnostics <svg
      class="chevron"
      viewBox="0 0 16 16"
      width="16"
      height="16"
      aria-hidden="true"
    >
      <path d="M6 4l4 4-4 4" />
    </svg></summary
  >
  {#if loaded.state === "loading"}
    <p>Loading…</p>
  {:else if loaded.state === "error"}
    <p>Diagnostics aren't available right now.</p>
    <button type="button" onclick={load}>Retry</button>
  {:else}
    {@const value = loaded.value}
    <p class="about">
      Flashboard {value.app_version} · history {bytesLabel(
        value.history_bytes,
      )},
      {countLabel(value.history_samples)} samples
    </p>
    <ul>
      {#each realSources(value.sources) as item (item.source)}
        <li>
          <span class="name">{sourceName(item.source)}</span>
          <span>{sourceStatus(item, settings.value.hour_cycle)}</span>
        </li>
      {/each}
    </ul>
    <p>
      Tray: {value.tray_available
        ? "available"
        : "not available, so the window stays a normal window"}
    </p>
    <button type="button" onclick={load}>Refresh</button>
  {/if}
</details>

<style>
  summary {
    justify-content: space-between;
    color: var(--color-text);
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-md-line);
  }

  ul {
    list-style: none;
    margin: var(--space-3) 0;
    padding: 0;
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }

  li {
    display: grid;
    gap: var(--space-1);
    padding-block: var(--space-2);
    border-bottom: var(--rule) solid var(--color-rule);
  }

  .name {
    font-weight: var(--text-sm-weight-strong);
  }

  .about {
    margin-top: var(--space-3);
  }

  .chevron {
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition-property: rotate;
    transition-duration: var(--duration-fast);
    transition-timing-function: var(--ease);
  }

  details[open] .chevron {
    rotate: 90deg;
  }

  button {
    margin-top: var(--space-3);
  }

  @media (min-width: 34rem) {
    li {
      grid-template-columns: 12rem 1fr;
      gap: var(--space-4);
    }
  }
</style>
