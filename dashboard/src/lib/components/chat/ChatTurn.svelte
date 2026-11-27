<!-- One turn of the chat (C1): the speaker in a gutter, the turn's content beside it, no
     bubble. A provider's name is set in the serif; "You" stays plain. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    role: "user" | "assistant";
    who: string;
    /** The answering model, under the name. */
    model?: string | null;
    /** When the turn arrived, "2:18 PM"; omitted when unknown. */
    time?: string | null;
    /** Still being written: rises in and stays open. */
    live?: boolean;
    children: Snippet;
  }

  const {
    role,
    who,
    model = null,
    time = null,
    live = false,
    children,
  }: Props = $props();
</script>

<article class="msg" class:live data-role={role}>
  <header class="who">
    <span class="name" class:serif={role === "assistant"}>{who}</span>
    {#if model}<span class="model">{model}</span>{/if}
    {#if time}<span class="time">{time}</span>{/if}
  </header>
  <div class="body">{@render children()}</div>
</article>

<style>
  .msg {
    display: grid;
    grid-template-columns: var(--chat-gutter) minmax(0, 1fr);
    gap: var(--space-6);
    padding-block: var(--space-6);
    border-top: var(--rule) solid var(--color-rule);
    animation: rise var(--duration-spring-firm) var(--ease-spring-firm) both;
  }
  .who {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .name {
    font-size: var(--text-sm);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-sm-line);
    overflow-wrap: anywhere;
  }
  .name.serif {
    font-family: var(--font-serif);
    font-size: var(--name-serif);
    font-weight: var(--title-xl-weight);
    line-height: var(--name-serif-line);
  }
  .model {
    color: var(--color-muted);
    font-size: var(--text-micro);
    font-weight: var(--text-sm-weight-strong);
    line-height: var(--text-micro-line);
    letter-spacing: 0.05em;
    text-transform: uppercase;
    overflow-wrap: anywhere;
  }
  .time {
    color: var(--color-faint);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
    font-variant-numeric: tabular-nums;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    color: var(--color-ink);
    font-size: var(--text-lg);
    line-height: var(--text-lg-line);
  }
  /* base.css styles every <p> as a muted caption; a turn's text is reading text. Kept at
     zero specificity for the <p> so a caller's own classes (route lines) still win. */
  .body :global(:where(p)) {
    color: var(--color-ink);
    font-size: var(--text-lg);
    line-height: var(--text-lg-line);
  }

  @keyframes rise {
    from {
      opacity: 0;
      translate: 0 var(--space-2);
    }
    to {
      opacity: 1;
      translate: 0 0;
    }
  }

  /* Narrow windows: the speaker goes above the turn. */
  @media (max-width: 47.99rem) {
    .msg {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--space-2);
    }
  }
</style>
