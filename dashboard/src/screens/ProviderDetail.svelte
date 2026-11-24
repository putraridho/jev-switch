<script lang="ts">
  import Meter from "../lib/components/primitives/Meter.svelte";
  import { router } from "../lib/router.svelte";
  import { usage } from "../lib/stores/usage.svelte";
  const {}: Record<string, string> = $props();
  const provider = $derived(
    router.route.name === "provider"
      ? usage.snapshot?.providers.find(
          (item) =>
            router.route.name === "provider" &&
            item.provider === router.route.provider,
        )
      : undefined,
  );
</script>

<section aria-label="Provider detail">
  <button
    onclick={() => {
      router.back();
    }}>Back</button
  >
  {#if provider}
    <h1>{provider.provider}</h1>
    {#if provider.usage.state === "known" || provider.usage.state === "stale"}
      {#if provider.usage.state === "stale"}<p>
          As of {new Date(provider.usage.reading.observed_at).toLocaleString()}
        </p>{/if}
      {#if provider.usage.reading.plan}<p>{provider.usage.reading.plan}</p>{/if}
      {#each provider.usage.reading.meters as meter (meter.id)}
        {#if meter.kind === "window"}
          <p>{meter.id}</p>
          <div class="value">{Math.round(meter.used_bp / 100)}%</div>
          <Meter
            usedBp={meter.used_bp}
            severity={meter.severity}
            label={`${provider.provider} ${meter.id}, ${meter.status}`}
          />
          <p>
            {meter.resets_at === null
              ? "Reset time unavailable"
              : `Resets ${new Date(meter.resets_at * 1000).toLocaleString()}`} · {meter.status}
          </p>
        {:else}<div class="value">
            {meter.currency}
            {(meter.total_micros / 1000000).toFixed(2)}
          </div>{/if}
      {/each}
    {:else if provider.usage.state === "pending"}<p>Checking…</p>
    {:else if provider.usage.state === "not_configured"}<p>Not configured</p>
    {:else if provider.usage.state === "unavailable"}<p>
        Unavailable: {provider.usage.reason.replaceAll("_", " ")}
      </p>
    {:else}<p>Usage unavailable for this provider</p>{/if}
    <h2>History</h2>
    <p>History is not available yet.</p>
    {#if provider.usage.state === "not_configured"}<p>
        Connect: <code
          >{provider.usage.setup === "use_gateway"
            ? "jev-claude"
            : "DEEPSEEK_API_KEY"}</code
        >
      </p>{/if}
  {:else}<h1>Provider</h1>
    <p>Checking…</p>{/if}
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
    text-transform: capitalize;
  }
  h2 {
    margin-top: var(--space-8);
  }
  .value {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
    font-variant-numeric: tabular-nums;
  }
  button {
    min-height: var(--hit-min);
    cursor: pointer;
  }
  code {
    font-family: var(--font-mono);
    background: var(--color-chip);
    padding: var(--space-2);
  }
</style>
