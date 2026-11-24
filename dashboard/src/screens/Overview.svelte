<script lang="ts">
  import Meter from "../lib/components/primitives/Meter.svelte";

  // A sample provider until the IPC seam (T07) feeds the real snapshot.
  const sample = {
    usedBp: 8300,
    severity: "warn" as const,
    label: "Claude Max 5-hour",
  };
</script>

<section aria-label="Overview" class="overview">
  <h1>Usage at a glance</h1>
  <div class="provider">
    <Meter
      usedBp={sample.usedBp}
      severity={sample.severity}
      label={sample.label}
    />
    <div class="name">Claude Max</div>
    <div class="value">83%</div>
  </div>
</section>

<style>
  .overview {
    padding: var(--space-12);
    background: var(--color-bg);
    color: var(--color-text);
    font-family: var(--font-sans);
  }

  h1 {
    margin: 0 0 var(--space-8);
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    font-weight: var(--title-xl-weight);
    line-height: var(--title-xl-line);
  }

  .provider {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    align-items: flex-start;
  }

  .name {
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-md-line);
  }

  .value {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
  }
</style>
