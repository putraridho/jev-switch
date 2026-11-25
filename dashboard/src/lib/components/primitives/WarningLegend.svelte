<script lang="ts">
  interface Props {
    /** The warning threshold in percent. */
    warnAtPct?: number;
  }

  const { warnAtPct = 80 }: Props = $props();
</script>

<!-- S17, S25: the header's key to the triangle. Below 800 px only the glyph shows; the
     words stay for screen readers. -->
<span class="legend">
  <svg viewBox="0 0 10 12" width="10" height="12" aria-hidden="true">
    <path d="M0 6 L10 0 L10 12 Z" />
  </svg>
  <span class="words">{warnAtPct}% warning mark</span>
</span>

<style>
  .legend {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    font-weight: var(--text-sm-weight-strong);
    color: var(--color-muted);
    white-space: nowrap;
  }
  path {
    fill: currentColor;
  }
  @media (max-width: 50rem) {
    .words {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
    }
  }
</style>
