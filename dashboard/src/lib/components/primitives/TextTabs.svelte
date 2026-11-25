<!-- Text tabs (S36): sections of a page. 36 high on a hairline, a 2 px ink indicator under
     the current one. Arrows, Home and End move and select; the group is one tab stop. -->
<script lang="ts" generics="Value extends string">
  import type { Snippet } from "svelte";

  interface Props {
    /** Names the tab list. */
    label: string;
    tabs: readonly { value: Value; label: string }[];
    value: Value;
    onchange: (value: Value) => void;
    /** The panel of the current tab. */
    children: Snippet<[Value]>;
  }

  const { label, tabs, value, onchange, children }: Props = $props();

  const uid = $props.id();
  let list = $state<HTMLElement | null>(null);

  function move(event: KeyboardEvent): void {
    const at = tabs.findIndex((tab) => tab.value === value);
    const last = tabs.length - 1;
    let next: number;
    if (event.key === "ArrowRight") next = at >= last ? 0 : at + 1;
    else if (event.key === "ArrowLeft") next = at <= 0 ? last : at - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = last;
    else return;
    const tab = tabs[next];
    if (tab === undefined) return;
    event.preventDefault();
    onchange(tab.value);
    list?.querySelectorAll("button")[next]?.focus();
  }
</script>

<div class="list" role="tablist" aria-label={label} bind:this={list}>
  {#each tabs as tab (tab.value)}
    <button
      type="button"
      role="tab"
      id="{uid}-{tab.value}"
      aria-selected={tab.value === value}
      aria-controls="{uid}-panel"
      tabindex={tab.value === value ? 0 : -1}
      onkeydown={move}
      onclick={() => {
        onchange(tab.value);
      }}>{tab.label}</button
    >
  {/each}
</div>
<div role="tabpanel" id="{uid}-panel" aria-labelledby="{uid}-{value}">
  {@render children(value)}
</div>

<style>
  .list {
    display: flex;
    flex-wrap: wrap;
    column-gap: var(--space-6);
    border-bottom: var(--rule) solid var(--color-rule);
  }

  button,
  button:hover:not(:disabled),
  button:active:not(:disabled) {
    min-height: var(--tab-h);
    padding: 0;
    background: none;
    border: 0;
    border-bottom: var(--tab-indicator) solid transparent;
    border-radius: 0;
    margin-bottom: calc(-1 * var(--rule));
    color: var(--color-text-2);
    scale: 1;
  }

  button:hover:not(:disabled) {
    color: var(--color-text);
  }

  button[aria-selected="true"],
  button[aria-selected="true"]:active:not(:disabled) {
    color: var(--color-text);
    border-bottom-color: var(--color-ink);
  }

  @media (forced-colors: active) {
    button[aria-selected="true"] {
      border-bottom-color: currentColor;
    }
  }
</style>
