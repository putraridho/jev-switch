<!-- A tool approval (C1). It takes the composer's place until answered, on the ink surface
     (the system's one raised surface), and unrolls up out of it. Enter allows once, Esc
     denies; focus lands on "Allow once" so the keyboard answer is one key away. -->
<script lang="ts">
  import { onMount } from "svelte";
  import type { ChatDecision } from "../../ipc/chat-bindings/ChatDecision";
  import type { ChatPermissionPrompt } from "../../ipc/chat-bindings/ChatPermissionPrompt";
  import { pushEscape } from "../../escape";

  interface Props {
    prompt: ChatPermissionPrompt;
    /** Who is asking, e.g. "Claude"; "The agent" when the route isn't known. */
    who: string;
    /** The mode that made it ask, e.g. "Ask mode". */
    modeLabel: string;
    onanswer: (decision: ChatDecision) => void;
  }

  const { prompt, who, modeLabel, onanswer }: Props = $props();

  let allow = $state<HTMLButtonElement | null>(null);
  onMount(() => {
    allow?.focus();
    return pushEscape("dialog", () => {
      onanswer("deny");
      return true;
    });
  });
</script>

<div class="approval" role="alertdialog" aria-label="Tool approval required">
  <div class="head">
    <h2><span class="who">{who}</span> wants to use {prompt.tool}</h2>
    <span class="mode">{modeLabel}</span>
  </div>
  <pre class="command">{prompt.summary}</pre>
  {#if prompt.blocked_path}
    <p class="blocked">Outside your working folder: {prompt.blocked_path}</p>
  {/if}
  <div class="actions">
    <button
      type="button"
      class="allow"
      bind:this={allow}
      onclick={() => {
        onanswer("allow");
      }}>Allow once</button
    >
    {#if prompt.can_always}
      <button
        type="button"
        class="ghost"
        onclick={() => {
          onanswer("allow_always");
        }}
        >Always allow{prompt.always_label
          ? ` ${prompt.always_label}`
          : ""}</button
      >
    {/if}
    <span class="spacer"></span>
    <button
      type="button"
      class="ghost"
      onclick={() => {
        onanswer("deny");
      }}>Deny <span class="key" aria-hidden="true">Esc</span></button
    >
  </div>
</div>

<style>
  .approval {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-dialog);
    background: var(--color-ink);
    color: var(--color-paper);
    animation: unroll var(--duration-spring-heavy) var(--ease-spring-heavy) both;
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-4);
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: var(--text-md-weight);
    line-height: var(--text-md-line);
  }
  .who {
    font-family: var(--font-serif);
    font-size: var(--text-title);
    line-height: var(--text-title-line);
  }
  .mode {
    color: var(--color-on-ink-muted);
    font-size: var(--text-micro);
  }
  .command {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-control);
    background: var(--color-ink-hover);
    color: var(--color-paper);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .blocked {
    color: var(--color-on-ink-alert);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
  .actions button {
    color: var(--color-paper);
    border-color: var(--color-on-ink-muted);
  }
  .actions button:hover:not(:disabled) {
    background: var(--color-ink-hover);
  }
  .actions .allow {
    color: var(--color-ink);
    background: var(--color-paper);
    border-color: var(--color-paper);
  }
  .actions .allow:hover:not(:disabled) {
    background: var(--color-chip);
  }
  .actions :focus-visible {
    outline-color: var(--color-paper);
  }
  .key {
    color: var(--color-on-ink-muted);
    font-size: var(--text-micro);
    font-weight: var(--text-md-weight);
  }

  @keyframes unroll {
    from {
      clip-path: inset(100% 0 0 0 round var(--radius-dialog));
      translate: 0 var(--space-2);
    }
    to {
      clip-path: inset(0 0 0 0 round var(--radius-dialog));
      translate: 0 0;
    }
  }
</style>
