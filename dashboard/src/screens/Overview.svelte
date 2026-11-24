<script lang="ts">
  import Meter from "../lib/components/primitives/Meter.svelte";
  import type { ReadingView } from "../lib/ipc/bindings/ReadingView";
  import type { Setup } from "../lib/ipc/bindings/Setup";
  import type { Unavailable } from "../lib/ipc/bindings/Unavailable";
  import type { UsageView } from "../lib/ipc/bindings/UsageView";
  import { announce } from "../lib/stores/announce.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { usage } from "../lib/stores/usage.svelte";
  import { router } from "../lib/router.svelte";

  const unavailable: Record<Unavailable, string> = {
    not_installed: "Not installed",
    not_logged_in: "Not logged in",
    offline: "Offline",
    timeout: "Timed out",
    rate_limited: "Rate limited",
    unauthorized: "Unauthorized",
    server_error: "Server error",
    protocol: "Unexpected response",
    io: "Connection error",
    internal: "Internal error",
  };
  const setup: Record<Setup, string> = {
    use_gateway: "Connect the gateway",
    add_deepseek_key: "Add a DeepSeek key",
  };

  function readingOf(u: UsageView): ReadingView | null {
    return u.state === "known" || u.state === "stale" ? u.reading : null;
  }

  async function refresh(): Promise<void> {
    const result = await usage.refresh();
    announce.say(result.status === "ok" ? "Usage refreshed" : "Refresh failed");
  }

  function reset(at: number | null): string {
    if (at === null) return "Reset time unavailable";
    const minutes = Math.max(0, Math.ceil((at * 1000 - clock.now) / 60_000));
    return minutes === 0 ? "Reset due" : `Resets in ${String(minutes)} min`;
  }
</script>

<section aria-label="Overview">
  <header>
    <h1>Usage at a glance</h1>
    <button onclick={refresh} disabled={usage.refreshing}>
      {usage.refreshing ? "Refreshing…" : "Refresh"}
    </button>
  </header>
  {#if usage.snapshot === null}
    <p>Checking…</p>
  {:else}
    {#each usage.snapshot.providers as provider (provider.provider)}
      <article class:muted={!provider.enabled}>
        <h2>
          <button
            class="provider-link"
            onclick={() => {
              router.go({ name: "provider", provider: provider.provider });
            }}
          >
            {provider.provider}
          </button>
        </h2>
        {#if !provider.enabled}
          <p>Hidden</p>
        {:else if provider.usage.state === "pending"}
          <p>Checking…</p>
        {:else if provider.usage.state === "unavailable"}
          <p>{unavailable[provider.usage.reason]}</p>
        {:else if provider.usage.state === "not_configured"}
          <p>{setup[provider.usage.setup]}</p>
        {:else if provider.usage.state === "unsupported"}
          <p>Usage unavailable for this provider</p>
        {:else}
          {#if readingOf(provider.usage)?.plan}<p>
              {readingOf(provider.usage)?.plan}
            </p>{/if}
          {#if provider.usage.state === "stale"}
            <p>
              As of {new Date(
                readingOf(provider.usage)?.observed_at ?? 0,
              ).toLocaleString()}
            </p>
          {/if}
          {#each (readingOf(provider.usage)?.meters ?? []).slice(0, 1) as meter (meter.id)}
            {#if meter.kind === "window"}
              <div class="value">{Math.round(meter.used_bp / 100)}%</div>
              <Meter
                usedBp={meter.used_bp}
                severity={meter.severity}
                label={`${provider.provider} usage, ${meter.status}, ${reset(meter.resets_at)}`}
              />
              <p>{reset(meter.resets_at)} · {meter.status}</p>
              <details>
                <summary>More details</summary>
                <p>
                  {(meter.used_bp / 100).toFixed(1)}% · {meter.resets_at ===
                  null
                    ? "Reset time unavailable"
                    : new Date(meter.resets_at * 1000).toLocaleString()} · {meter.status}
                  · observed {new Date(
                    readingOf(provider.usage)?.observed_at ?? 0,
                  ).toLocaleString()}
                </p>
              </details>
            {:else}
              <div class="value">
                {meter.currency}
                {(meter.total_micros / 1000000).toFixed(2)}
              </div>
              <p>{meter.usable ? "Available" : "Unavailable"}</p>
            {/if}
          {/each}
        {/if}
      </article>
    {/each}
  {/if}
</section>

<style>
  section {
    padding: var(--space-6) var(--space-4);
    max-width: 40rem;
    margin-inline: auto;
    color: var(--color-text);
    font-family: var(--font-sans);
  }
  header {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--space-4);
  }
  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
    font-weight: var(--title-xl-weight);
  }
  article {
    padding-block: var(--space-6);
    border-bottom: var(--rule) solid var(--color-rule);
  }
  h2 {
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
  }
  .provider-link {
    text-transform: capitalize;
  }
  .value {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
    font-variant-numeric: tabular-nums;
  }
  .muted {
    color: var(--color-muted);
  }
</style>
