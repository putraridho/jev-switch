<script lang="ts">
  import type { DetailRow } from "./display";

  interface Props {
    rows: readonly DetailRow[];
  }

  const { rows }: Props = $props();
</script>

<dl>
  {#each rows as row (row.label)}
    <div>
      <dt>{row.label}</dt>
      <dd class:alert={row.alert === true}>{row.value}</dd>
    </div>
  {/each}
</dl>

<style>
  /* S23: a top rule, 12 padding, 13/20 rows, label left and value right. */
  dl {
    margin: 0;
    padding: var(--space-3);
    border-top: var(--rule) solid var(--color-rule);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }
  div {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
  }
  dt {
    color: var(--color-muted);
  }
  dd {
    margin: 0;
    text-align: end;
    color: var(--color-ink);
    font-variant-numeric: tabular-nums;
  }
  dd.alert {
    color: var(--color-alert);
  }
</style>
