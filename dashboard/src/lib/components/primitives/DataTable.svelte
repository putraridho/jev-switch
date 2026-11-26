<!-- S43: hairline rows, right aligned numbers, no fills. An ink rule closes the header; the
     sorted column's header is ink with an arrow. -->
<script lang="ts">
  import type { SortDirection, TableCell, TableColumn } from "./display";

  interface Props {
    /** The table's accessible name. */
    caption: string;
    columns: readonly TableColumn[];
    rows: readonly (readonly TableCell[])[];
    sortIndex?: number;
    sortDirection?: SortDirection;
    /** When given, each header is a button that asks for that column's order. */
    onsort?: (index: number) => void;
  }

  const {
    caption,
    columns,
    rows,
    sortIndex,
    sortDirection = "ascending",
    onsort,
  }: Props = $props();
</script>

{#snippet heading(label: string, sorted: boolean)}
  <span>{label}</span>
  {#if sorted}
    <svg
      class="arrow"
      class:down={sortDirection === "descending"}
      viewBox="0 0 12 12"
      width="12"
      height="12"
      aria-hidden="true"
    >
      <path d="M6 10 V2 M3 5 L6 2 L9 5" />
    </svg>
  {/if}
{/snippet}

<table>
  <caption class="hidden">{caption}</caption>
  <thead>
    <tr>
      {#each columns as column, i (column.label)}
        <th
          scope="col"
          class:numeric={column.numeric}
          class:sorted={i === sortIndex}
          aria-sort={i === sortIndex ? sortDirection : undefined}
        >
          {#if onsort}
            <button
              type="button"
              onclick={() => {
                onsort(i);
              }}>{@render heading(column.label, i === sortIndex)}</button
            >
          {:else}
            <span class="plain"
              >{@render heading(column.label, i === sortIndex)}</span
            >
          {/if}
        </th>
      {/each}
    </tr>
  </thead>
  <tbody>
    {#each rows as row, r (r)}
      <tr>
        {#each columns as column, i (column.label)}
          {@const cell = row[i]}
          <td
            class:numeric={column.numeric}
            class:strong={column.tone === "strong"}
            class:muted={column.tone === "muted" || cell?.tone === "muted"}
            class:alert={cell?.tone === "alert"}>{cell?.text ?? ""}</td
          >
        {/each}
      </tr>
    {/each}
  </tbody>
</table>

<style>
  table {
    inline-size: 100%;
    border-collapse: collapse;
  }
  .hidden {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  th {
    block-size: var(--row-h);
    padding: 0 var(--space-3) 0 0;
    border-block-end: var(--rule) solid var(--color-ink);
    color: var(--color-muted);
    font-size: var(--text-micro);
    font-weight: var(--text-micro-weight);
    line-height: var(--text-micro-line);
    text-align: start;
  }
  th.sorted {
    color: var(--color-ink);
    font-weight: var(--text-md-weight-strong);
  }
  th:last-child,
  td:last-child {
    padding-inline-end: 0;
  }
  th button,
  .plain {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  th button {
    justify-content: flex-start;
    min-inline-size: 0;
    min-block-size: var(--hit-min);
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    white-space: nowrap;
  }
  td {
    block-size: var(--table-row-h);
    padding: var(--space-1) var(--space-3) var(--space-1) 0;
    border-block-end: var(--rule) solid var(--color-rule);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }
  .numeric {
    text-align: end;
  }
  td.numeric {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  td.strong {
    font-weight: var(--text-md-weight-strong);
  }
  td.muted {
    color: var(--color-muted);
  }
  td.alert {
    color: var(--color-alert);
  }
  .arrow {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .down {
    transform: rotate(180deg);
  }
  /* Five columns on a 320 px window: tighter gutters and wrapping, never a side scroll. */
  @media (max-width: 30rem) {
    th,
    td {
      padding-inline-end: var(--space-2);
    }
    td {
      font-size: var(--text-micro);
      line-height: var(--text-micro-line);
    }
    th button,
    .plain {
      gap: 0;
    }
  }
</style>
