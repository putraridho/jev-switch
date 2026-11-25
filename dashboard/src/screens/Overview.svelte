<script lang="ts">
  import MeterRow from "../lib/components/primitives/MeterRow.svelte";
  import CommandLine from "../lib/components/primitives/CommandLine.svelte";
  import { refreshAll, retryProvider } from "../lib/actions";
  import {
    retryable,
    planName,
    providerName,
    setupCopy,
    staleNote,
    unavailableCopy,
    unsupportedLine,
  } from "../lib/copy";
  import { money } from "../lib/format";
  import type { ReadingView } from "../lib/ipc/bindings/ReadingView";
  import type { UsageView } from "../lib/ipc/bindings/UsageView";
  import { arrange } from "../lib/overview";
  import { router } from "../lib/router.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { usage } from "../lib/stores/usage.svelte";

  function readingOf(u: UsageView): ReadingView | null {
    return u.state === "known" || u.state === "stale" ? u.reading : null;
  }

  const arranged = $derived(
    usage.snapshot === null ? null : arrange(usage.snapshot),
  );

  // A card that moves must not take the keyboard focus with it: moving a DOM node drops
  // focus in WebKit, so the screen puts it back (DESIGN 9.3).
  let section = $state<HTMLElement | null>(null);
  let focused: HTMLElement | null = null;
  const order = $derived(
    arranged === null ? "" : arranged.cards.map((card) => card.provider).join(),
  );
  $effect.pre(() => {
    if (order === "") return;
    const active = document.activeElement;
    focused =
      active instanceof HTMLElement && section?.contains(active) === true
        ? active
        : null;
  });
  $effect(() => {
    if (order === "") return;
    if (focused?.isConnected === true && document.activeElement !== focused) {
      focused.focus();
    }
  });
</script>

<section aria-label="Overview" bind:this={section}>
  <header>
    <h1 id="view-title" tabindex="-1">Usage at a glance</h1>
    <button onclick={refreshAll} disabled={usage.refreshing}>
      {usage.refreshing ? "Refreshing…" : "Refresh"}
    </button>
  </header>
  {#if arranged === null}
    <p>Checking…</p>
  {:else}
    {#each arranged.cards as provider (provider.provider)}
      {@const reading = readingOf(provider.usage)}
      {@const note = staleNote(
        provider.provider,
        provider.usage,
        clock.now,
        settings.value.hour_cycle,
      )}
      <article>
        <h2>
          <button
            class="provider-link"
            onclick={() => {
              router.go({ name: "provider", provider: provider.provider });
            }}
          >
            <span>{providerName[provider.provider]}</span>
            {#if reading?.plan}<span class="plan">{planName(reading.plan)}</span
              >{/if}
          </button>
        </h2>
        {#if provider.usage.state === "pending"}
          <p>Checking…</p>
        {:else if provider.usage.state === "unavailable"}
          {@const copy = unavailableCopy(
            provider.usage.reason,
            provider.provider,
          )}
          <p>{copy.text}</p>
          {#if copy.command}<CommandLine command={copy.command} />{/if}
          {#if retryable(provider.usage.reason)}
            <button
              type="button"
              class="retry"
              onclick={() =>
                retryProvider(
                  provider.provider,
                  providerName[provider.provider],
                )}>Retry</button
            >
          {/if}
        {:else if provider.usage.state === "not_configured"}
          {@const copy = setupCopy(provider.usage.setup, provider.provider)}
          <p>{copy.text}</p>
          {#if copy.command}<CommandLine command={copy.command} />{/if}
        {:else if reading}
          {#if note}
            <div class="note-row">
              <p class="note">{note}</p>
              {#if provider.usage.state === "stale" && provider.usage.cause.kind === "failing"}
                <button
                  type="button"
                  onclick={() =>
                    retryProvider(
                      provider.provider,
                      providerName[provider.provider],
                    )}>Retry</button
                >
              {/if}
            </div>
          {/if}
          {#each reading.meters as meter (meter.id)}
            {#if meter.kind === "window"}
              <MeterRow
                provider={provider.provider}
                {meter}
                observedAt={reading.observed_at}
              />
            {:else}
              <div class="balance">
                {money(meter.total_micros, meter.currency)}
              </div>
              <p>{meter.usable ? "Available" : "Not usable"}</p>
            {/if}
          {/each}
        {/if}
      </article>
    {/each}
    {#if arranged.unsupported.length > 0}
      <p class="unsupported">
        {unsupportedLine(arranged.unsupported.map((item) => item.provider))}
      </p>
    {/if}
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
    display: inline-flex;
    gap: var(--space-2);
    align-items: baseline;
    border: none;
    padding-inline: 0;
    font-weight: var(--text-md-weight-strong);
  }
  .plan {
    color: var(--color-text-2);
    font-weight: var(--text-md-weight);
  }
  .balance {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
    font-variant-numeric: tabular-nums;
  }
  .unsupported {
    padding-block: var(--space-6);
  }
  .note-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }
  .retry {
    margin-top: var(--space-2);
  }
</style>
