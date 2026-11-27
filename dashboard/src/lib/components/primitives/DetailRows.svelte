<script lang="ts">
  import type { DetailRow } from "./display";

  interface Props {
    rows: readonly DetailRow[];
    /** On the ink surface, as a disclosure floating over the page (S37's tooltip look). */
    floating?: boolean;
  }

  const { rows, floating = false }: Props = $props();
</script>

<dl class:floating>
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

  /* Floating: the system's one raised surface, the ink tooltip. Ink fill, paper text, radius
     6, no border and no shadow; labels and the alert value take the on-ink colours. */
  .floating {
    padding: var(--space-2) var(--space-3);
    border-top: 0;
    border-radius: var(--radius-control);
    background: var(--color-ink);
    font-size: var(--text-micro);
    line-height: var(--text-sm-line);
  }
  .floating dt {
    color: var(--color-on-ink-muted);
  }
  .floating dd {
    color: var(--color-paper);
  }
  .floating dd.alert {
    color: var(--color-on-ink-alert);
  }

  /* The tooltip keeps its own ink-and-paper pair in forced colours, as the dot tooltip does. */
  @media (forced-colors: active) {
    .floating {
      forced-color-adjust: none;
    }
  }
</style>
