<!-- The overflow menu (S38): a hairline box, 32 px items, separators, an optional key hint,
     a destructive item in the alert colour. Opens from its own button; arrows move, `Esc`
     and an outside click close, and focus returns to the button. -->
<script lang="ts">
  import { tick } from "svelte";
  import { pushEscape } from "../../escape";
  import type { MenuItem } from "./display";

  interface Props {
    /** The button's visible word. */
    label: string;
    items: readonly MenuItem[];
  }

  const { label, items }: Props = $props();

  const uid = $props.id();
  let open = $state(false);
  let button = $state<HTMLButtonElement | null>(null);
  let menu = $state<HTMLElement | null>(null);

  function entries(): readonly HTMLElement[] {
    const found: HTMLElement[] = [];
    for (const node of menu?.querySelectorAll('[role="menuitem"]') ?? []) {
      if (node instanceof HTMLElement) found.push(node);
    }
    return found;
  }

  async function show(): Promise<void> {
    open = true;
    await tick();
    entries()[0]?.focus();
  }

  function close(): void {
    open = false;
    button?.focus();
  }

  function move(event: KeyboardEvent): void {
    const list = entries();
    const at = list.findIndex((item) => item === document.activeElement);
    const last = list.length - 1;
    let next: number;
    if (event.key === "ArrowDown") next = at >= last ? 0 : at + 1;
    else if (event.key === "ArrowUp") next = at <= 0 ? last : at - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = last;
    else {
      if (event.key === "Tab") open = false;
      return;
    }
    event.preventDefault();
    list[next]?.focus();
  }

  $effect(() => {
    if (!open) return;
    function outside(event: PointerEvent): void {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (menu?.contains(target) === true) return;
      if (button?.contains(target) === true) return;
      open = false;
    }
    window.addEventListener("pointerdown", outside);
    const pop = pushEscape("disclosure", () => {
      close();
      return true;
    });
    return () => {
      window.removeEventListener("pointerdown", outside);
      pop();
    };
  });
</script>

<div class="menu">
  <button
    type="button"
    bind:this={button}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? uid : undefined}
    onclick={() => {
      if (open) close();
      else void show();
    }}>{label}</button
  >
  {#if open}
    <div
      class="box"
      role="menu"
      id={uid}
      aria-label={label}
      tabindex="-1"
      bind:this={menu}
      onkeydown={move}
    >
      {#each items as item (item.label)}
        {#if item.separated}<div class="separator" role="separator"></div>{/if}
        <button
          type="button"
          role="menuitem"
          tabindex="-1"
          class:destructive={item.destructive}
          onclick={() => {
            close();
            item.onselect();
          }}
        >
          <span>{item.label}</span>
          {#if item.shortcut !== undefined}<kbd aria-hidden="true"
              >{item.shortcut}</kbd
            >{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
    display: inline-block;
  }

  .box {
    position: absolute;
    top: calc(100% + var(--space-1));
    left: 0;
    z-index: 3;
    width: min(var(--menu-w), calc(100vw - 2 * var(--space-4)));
    padding: var(--space-1);
    background: var(--color-paper);
    border: var(--outline) solid var(--color-border-strong);
    border-radius: var(--radius-dialog);
  }

  .box button {
    display: flex;
    justify-content: space-between;
    width: 100%;
    padding-inline: var(--space-3);
    border: 0;
    border-radius: var(--radius-small);
    text-align: left;
    scale: 1;
  }

  .box button:focus-visible {
    background: var(--color-hover);
    outline-offset: calc(-1 * var(--focus-ring));
  }

  .destructive {
    color: var(--color-alert);
  }

  kbd {
    color: var(--color-text-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: var(--text-micro-weight);
  }

  .separator {
    height: var(--rule);
    margin-block: var(--space-1);
    background: var(--color-rule);
  }

  @media (forced-colors: active) {
    .separator {
      border-top: var(--rule) solid currentColor;
    }
  }
</style>
