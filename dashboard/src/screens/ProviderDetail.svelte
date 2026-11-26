<script lang="ts">
  import { retryProvider } from "../lib/actions";
  import BalanceFigure from "../lib/components/primitives/BalanceFigure.svelte";
  import CommandLine from "../lib/components/primitives/CommandLine.svelte";
  import ErrorState from "../lib/components/primitives/ErrorState.svelte";
  import IconButton from "../lib/components/primitives/IconButton.svelte";
  import LoadingWaffle from "../lib/components/primitives/LoadingWaffle.svelte";
  import MeterRow from "../lib/components/primitives/MeterRow.svelte";
  import NoSource from "../lib/components/primitives/NoSource.svelte";
  import PlanTag from "../lib/components/primitives/PlanTag.svelte";
  import StaleRow from "../lib/components/primitives/StaleRow.svelte";
  import HistorySection from "../lib/components/shell/HistorySection.svelte";
  import {
    retryable,
    planName,
    providerName,
    setupCopy,
    staleNote,
    unavailableCopy,
  } from "../lib/copy";
  import { money } from "../lib/format";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  import { router } from "../lib/router.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { usage } from "../lib/stores/usage.svelte";
  import { viewport } from "../lib/viewport.svelte";

  const provider = $derived(
    router.route.name === "provider"
      ? usage.snapshot?.providers.find(
          (item) =>
            router.route.name === "provider" &&
            item.provider === router.route.provider,
        )
      : undefined,
  );
  const size = $derived(viewport.wide ? "large" : "medium");
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
  const plan = $derived(
    provider?.usage.state === "known" || provider?.usage.state === "stale"
      ? provider.usage.reading.plan
      : null,
  );

  function retry(key: ProviderKey): void {
    void retryProvider(key, providerName[key]);
  }
</script>

<section class="detail" aria-label="Provider detail">
  <div class="title">
    <IconButton
      label="Back"
      icon="back"
      onclick={() => {
        router.back();
      }}
    />
    <h1 id="view-title" tabindex="-1">
      {provider ? providerName[provider.provider] : "Provider"}
    </h1>
    {#if plan}<PlanTag plan={planName(plan)} />{/if}
  </div>
  {#if provider}
    {@const state = provider.usage}
    {@const key = provider.provider}
    {#if state.state === "known" || state.state === "stale"}
      {@const reading = state.reading}
      {@const failing =
        state.state === "stale" && state.cause.kind === "failing"}
      {#if note}
        <div class="note-row">
          <StaleRow text={note} />
          {#if failing}
            <button
              type="button"
              class="small"
              onclick={() => {
                retry(key);
              }}>Retry</button
            >
          {/if}
        </div>
      {/if}
      <div class="readings">
        {#each reading.meters as meter (meter.id)}
          {#if meter.kind === "window"}
            <MeterRow
              provider={key}
              {meter}
              observedAt={reading.observed_at}
              {size}
              stale={failing}
            />
          {:else}
            <BalanceFigure
              amount={money(meter.total_micros, meter.currency)}
              caption={meter.usable ? "Available" : "Not usable"}
              low={meter.severity === "warn" || meter.severity === "critical"}
            />
          {/if}
        {/each}
      </div>
      <HistorySection provider={key} meters={reading.meters} />
    {:else if state.state === "pending"}
      <LoadingWaffle {size} announcement="Checking {providerName[key]}" />
    {:else if state.state === "not_configured"}
      {@const copy = setupCopy(state.setup, key)}
      {#snippet command()}<CommandLine command={copy.command ?? ""} />{/snippet}
      <ErrorState
        message={copy.text}
        {...copy.command === undefined ? {} : { command }}
      />
    {:else if state.state === "unavailable"}
      {@const copy = unavailableCopy(state.reason, key)}
      {#snippet command()}<CommandLine command={copy.command ?? ""} />{/snippet}
      {#snippet action()}<button
          type="button"
          onclick={() => {
            retry(key);
          }}>Retry</button
        >{/snippet}
      <ErrorState
        message={copy.text}
        {...copy.command === undefined ? {} : { command }}
        {...retryable(state.reason) ? { action } : {}}
      />
    {:else}
      <NoSource names={[providerName[key]]} />
    {/if}
  {:else}
    <LoadingWaffle {size} />
  {/if}
</section>

<style>
  .detail {
    box-sizing: border-box;
    max-width: 40rem;
    margin-inline: auto;
    padding: var(--space-6) var(--space-4) var(--space-8);
    font-family: var(--font-sans);
    color: var(--color-text);
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-block-end: var(--space-6);
  }
  h1 {
    margin: 0;
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
    font-weight: var(--title-xl-weight);
  }
  .note-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-block-end: var(--space-2);
  }
  .readings {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-4) var(--space-12);
  }

  @media (min-width: 40rem) {
    .detail {
      padding: var(--space-14) var(--space-6) var(--space-8);
    }
  }

  @media (min-width: 56.25rem) {
    .detail {
      max-width: 64rem;
    }
    .readings {
      grid-template-columns: repeat(auto-fill, 16rem);
    }
  }
</style>
