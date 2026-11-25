<script lang="ts">
  import { retryProvider } from "../lib/actions";
  import CommandLine from "../lib/components/primitives/CommandLine.svelte";
  import MeterRow from "../lib/components/primitives/MeterRow.svelte";
  import HistorySection from "../lib/components/shell/HistorySection.svelte";
  import {
    retryable,
    planName,
    providerName,
    setupCopy,
    staleNote,
    unavailableCopy,
    unsupportedCopy,
  } from "../lib/copy";
  import { money } from "../lib/format";
  import { router } from "../lib/router.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { usage } from "../lib/stores/usage.svelte";

  const provider = $derived(
    router.route.name === "provider"
      ? usage.snapshot?.providers.find(
          (item) =>
            router.route.name === "provider" &&
            item.provider === router.route.provider,
        )
      : undefined,
  );
  const note = $derived(
    provider === undefined
      ? null
      : staleNote(
          provider.provider,
          provider.usage,
          clock.now,
          settings.value.hour_cycle,
        ),
  );
</script>

<section aria-label="Provider detail">
  <button
    onclick={() => {
      router.back();
    }}>Back</button
  >
  {#if provider}
    <h1 id="view-title" tabindex="-1">{providerName[provider.provider]}</h1>
    {#if provider.usage.state === "known" || provider.usage.state === "stale"}
      {@const reading = provider.usage.reading}
      {#if reading.plan}<p>{planName(reading.plan)}</p>{/if}
      {#if note}<p class="note">{note}</p>{/if}
      {#if provider.usage.state === "stale" && provider.usage.cause.kind === "failing"}
        <button
          type="button"
          class="retry"
          onclick={() =>
            retryProvider(provider.provider, providerName[provider.provider])}
          >Retry</button
        >
      {/if}
      {#each reading.meters as meter (meter.id)}
        {#if meter.kind === "window"}
          <MeterRow
            provider={provider.provider}
            {meter}
            observedAt={reading.observed_at}
          />
        {:else}
          <div class="balance">{money(meter.total_micros, meter.currency)}</div>
          <p>{meter.usable ? "Available" : "Not usable"}</p>
        {/if}
      {/each}
    {:else if provider.usage.state === "pending"}
      <p>Checking…</p>
    {:else if provider.usage.state === "not_configured"}
      {@const copy = setupCopy(provider.usage.setup, provider.provider)}
      <p>{copy.text}</p>
      {#if copy.command}<CommandLine command={copy.command} />{/if}
    {:else if provider.usage.state === "unavailable"}
      {@const copy = unavailableCopy(provider.usage.reason, provider.provider)}
      <p>{copy.text}</p>
      {#if copy.command}<CommandLine command={copy.command} />{/if}
      {#if retryable(provider.usage.reason)}
        <button
          type="button"
          class="retry"
          onclick={() =>
            retryProvider(provider.provider, providerName[provider.provider])}
          >Retry</button
        >
      {/if}
    {:else}
      <p>{unsupportedCopy(provider.provider)}</p>
    {/if}
    {#if provider.usage.state === "known" || provider.usage.state === "stale"}
      <HistorySection
        provider={provider.provider}
        meters={provider.usage.reading.meters}
      />
    {/if}
  {:else}
    <h1 id="view-title" tabindex="-1">Provider</h1>
    <p>Checking…</p>
  {/if}
</section>

<style>
  section {
    padding: var(--space-6) var(--space-4);
    max-width: 40rem;
    margin-inline: auto;
    font-family: var(--font-sans);
    color: var(--color-text);
  }
  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
  }
  .retry {
    margin-block: var(--space-2);
  }
  .balance {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
    font-variant-numeric: tabular-nums;
  }
</style>
