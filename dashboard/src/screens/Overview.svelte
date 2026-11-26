<script lang="ts">
  import BalanceFigure from "../lib/components/primitives/BalanceFigure.svelte";
  import CommandLine from "../lib/components/primitives/CommandLine.svelte";
  import EmptyState from "../lib/components/primitives/EmptyState.svelte";
  import ErrorState from "../lib/components/primitives/ErrorState.svelte";
  import LoadingWaffle from "../lib/components/primitives/LoadingWaffle.svelte";
  import MeterRow from "../lib/components/primitives/MeterRow.svelte";
  import NoSource from "../lib/components/primitives/NoSource.svelte";
  import PlanTag from "../lib/components/primitives/PlanTag.svelte";
  import Section from "../lib/components/primitives/Section.svelte";
  import StaleRow from "../lib/components/primitives/StaleRow.svelte";
  import { retryProvider } from "../lib/actions";
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
  import type { ProviderView } from "../lib/ipc/bindings/ProviderView";
  import { arrange, hasWaffle } from "../lib/overview";
  import { router } from "../lib/router.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { usage } from "../lib/stores/usage.svelte";
  import { viewport } from "../lib/viewport.svelte";

  const arranged = $derived(
    usage.snapshot === null ? null : arrange(usage.snapshot),
  );
  const meters = $derived(arranged?.cards.filter(hasWaffle) ?? []);
  const others = $derived(
    arranged?.cards.filter((card) => !hasWaffle(card)) ?? [],
  );
  const size = $derived(viewport.wide ? "large" : "medium");

  function retry(provider: ProviderKey): void {
    void retryProvider(provider, providerName[provider]);
  }

  // A card that moves must not take the keyboard focus with it: moving a DOM node drops
  // focus in WebKit, so the screen puts it back (DESIGN 9.3). A card that changes group is
  // drawn anew, so focus goes to that provider's heading.
  let section = $state<HTMLElement | null>(null);
  let focused: HTMLElement | null = null;
  let focusedProvider: string | undefined;
  const order = $derived(
    [...meters, ...others].map((card) => card.provider).join() +
      `|${String(meters.length)}`,
  );
  $effect.pre(() => {
    if (order === "") return;
    const active = document.activeElement;
    focused =
      active instanceof HTMLElement && section?.contains(active) === true
        ? active
        : null;
    const owner = focused?.closest("[data-provider]");
    focusedProvider =
      owner instanceof HTMLElement ? owner.dataset["provider"] : undefined;
  });
  $effect(() => {
    if (order === "" || focused === null) return;
    if (focused.isConnected) {
      if (document.activeElement !== focused) focused.focus();
    } else if (focusedProvider !== undefined) {
      const heading = section?.querySelector(
        `[data-provider="${focusedProvider}"] .provider-link`,
      );
      if (heading instanceof HTMLElement) heading.focus();
    }
  });
</script>

{#snippet card(provider: ProviderView, level: "h2" | "h3")}
  {@const state = provider.usage}
  {@const key = provider.provider}
  {@const reading =
    state.state === "known" || state.state === "stale" ? state.reading : null}
  {@const note = staleNote(key, state, clock.now, settings.value.hour_cycle)}
  {@const failing = state.state === "stale" && state.cause.kind === "failing"}
  <article data-provider={key}>
    <svelte:element this={level} class="heading">
      <button
        type="button"
        class="provider-link text"
        onclick={() => {
          router.go({ name: "provider", provider: key });
        }}
      >
        <span>{providerName[key]}</span>
        {#if reading?.plan}<PlanTag plan={planName(reading.plan)} />{/if}
      </button>
    </svelte:element>
    {#if state.state === "pending"}
      <LoadingWaffle {size} announcement="Checking {providerName[key]}" />
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
    {:else if state.state === "not_configured"}
      {@const copy = setupCopy(state.setup, key)}
      {#snippet command()}<CommandLine command={copy.command ?? ""} />{/snippet}
      <ErrorState
        message={copy.text}
        {...copy.command === undefined ? {} : { command }}
      />
    {:else if reading}
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
    {/if}
  </article>
{/snippet}

<section class="overview" aria-label="Overview" bind:this={section}>
  <h1 id="view-title" tabindex="-1">Usage at a glance</h1>
  {#if arranged === null}
    <LoadingWaffle {size} />
  {:else if arranged.cards.length === 0 && arranged.unsupported.length === 0}
    {#snippet openSettings()}<button
        type="button"
        onclick={() => {
          router.go({ name: "settings" });
        }}>Open Settings</button
      >{/snippet}
    <EmptyState
      title="Nothing to watch yet."
      body="Every provider is hidden. Turn one on in Settings."
      action={openSettings}
    />
  {:else}
    {#if meters.length > 0}
      <div class="waffles">
        {#each meters as provider (provider.provider)}
          {@render card(provider, "h2")}
        {/each}
      </div>
    {/if}
    {#if others.length > 0 || arranged.unsupported.length > 0}
      <Section label="Other providers" secondary={meters.length > 0}>
        <div class="others">
          {#each others as provider (provider.provider)}
            {@render card(provider, "h3")}
          {/each}
          <NoSource
            names={arranged.unsupported.map(
              (item) => providerName[item.provider],
            )}
          />
        </div>
      </Section>
    {/if}
  {/if}
</section>

<style>
  .overview {
    box-sizing: border-box;
    max-width: 90rem;
    margin-inline: auto;
    padding: var(--space-6) var(--space-4) var(--space-8);
    color: var(--color-text);
    font-family: var(--font-sans);
  }
  h1 {
    margin: 0 0 var(--space-6);
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    font-weight: var(--text-md-weight-strong);
  }
  .waffles {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-8) var(--space-12);
  }
  .waffles > article {
    flex: 1 1 100%;
    min-width: 0;
  }
  .others {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 16rem), 1fr));
    align-items: start;
    gap: var(--space-8) var(--space-12);
  }
  article {
    display: grid;
    align-content: start;
    justify-items: start;
    gap: var(--space-2);
    min-width: 0;
  }
  .heading {
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--text-md-line);
  }
  .provider-link {
    display: inline-flex;
    gap: var(--space-2);
    align-items: baseline;
    padding-inline: 0;
    border: none;
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
  }
  .note-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }
  .readings {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    justify-self: stretch;
    gap: var(--space-4) var(--space-12);
  }

  @media (min-width: 40rem) {
    .overview {
      padding: var(--space-14) var(--space-6) var(--space-8);
    }
    .waffles > article {
      flex: 1 1 17rem;
    }
  }

  /* S50: 256 columns side by side, one per window. */
  @media (min-width: 56.25rem) {
    .waffles > article {
      flex: 0 0 auto;
    }
    .readings {
      grid-auto-flow: column;
      grid-template-columns: none;
      grid-auto-columns: 16rem;
      justify-self: start;
    }
  }

  @media (min-width: 80rem) {
    .overview {
      padding-inline: var(--space-18);
    }
  }
</style>
