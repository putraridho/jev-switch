<script lang="ts">
  import StatusBadge from "./StatusBadge.svelte";

  interface Props {
    message: string;
    /** A threshold toast: alert rule and triangle. Confirmations stay ink. */
    warning?: boolean;
    ondismiss: () => void;
    /** Hovering pauses the timer; leaving restarts it. */
    durationMs?: number;
  }

  const {
    message,
    warning = false,
    ondismiss,
    durationMs = 4000,
  }: Props = $props();

  let paused = $state(false);
  $effect(() => {
    // A new message restarts the timer.
    if (paused || message === "") return;
    const timer = setTimeout(ondismiss, durationMs);
    return () => {
      clearTimeout(timer);
    };
  });
</script>

<!-- S40: one at a time, bottom centre; the caller keeps only the newest. -->
<div
  class="toast"
  class:warning
  role="status"
  onpointerenter={() => {
    paused = true;
  }}
  onpointerleave={() => {
    paused = false;
  }}
>
  <StatusBadge kind={warning ? "near_limit" : "healthy"} label={message} />
</div>

<style>
  .toast {
    position: fixed;
    inset-block-end: var(--space-6);
    inset-inline: var(--space-4);
    z-index: 2;
    display: flex;
    align-items: center;
    box-sizing: border-box;
    max-inline-size: 25rem;
    min-block-size: var(--space-12);
    margin-inline: auto;
    padding-inline: var(--space-4);
    overflow: hidden;
    background: var(--color-paper);
    border: var(--outline) solid var(--color-faint);
    border-radius: var(--radius-control);
  }
  .toast.warning {
    border-color: var(--color-alert);
  }
</style>
