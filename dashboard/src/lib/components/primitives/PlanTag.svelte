<script lang="ts">
  interface Props {
    plan: string;
  }

  const { plan }: Props = $props();
</script>

<span>{plan}</span>

<style>
  /* S41: 12/500 muted capitals after the provider name, on its baseline. */
  span {
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
    font-weight: var(--text-sm-weight-strong);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-muted);
    white-space: nowrap;
  }
</style>
