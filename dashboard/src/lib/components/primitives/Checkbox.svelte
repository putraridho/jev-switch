<!-- The Waffle checkbox (S35): 18 px box, ink when checked or mixed. The whole row is the
     label. For choosing items from a list; an on/off setting is a Switch. -->
<script lang="ts">
  interface Props {
    label: string;
    checked: boolean;
    /** "Some of them": drawn as a dash. */
    mixed?: boolean;
    hint?: string;
    disabled?: boolean;
    onchange: (checked: boolean) => void;
  }

  const {
    label,
    checked,
    mixed = false,
    hint,
    disabled = false,
    onchange,
  }: Props = $props();

  const uid = $props.id();
  let input = $state<HTMLInputElement | null>(null);

  $effect(() => {
    if (input !== null) input.indeterminate = mixed;
  });
</script>

<label class="waffle-checkbox">
  <span class="control">
    <input
      bind:this={input}
      type="checkbox"
      {checked}
      {disabled}
      aria-labelledby="{uid}-name"
      aria-describedby={hint === undefined ? undefined : `${uid}-hint`}
      onchange={(event) => {
        onchange(event.currentTarget.checked);
      }}
    />
    <span class="box" aria-hidden="true">
      <svg viewBox="0 0 18 18" width="18" height="18" fill="none">
        <path class="tick" d="M5 9.5 L7.75 12 L13 6.25" />
        <path class="dash" d="M5 9 L13 9" />
      </svg>
    </span>
  </span>
  <span class="words">
    <span class="name" class:strong={hint !== undefined} id="{uid}-name"
      >{label}</span
    >
    {#if hint !== undefined}<span class="hint" id="{uid}-hint">{hint}</span
      >{/if}
  </span>
</label>

<style>
  .waffle-checkbox {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    min-height: var(--hit-min);
    padding-block: calc((var(--hit-min) - var(--text-md-line)) / 2);
    cursor: pointer;
  }

  .control {
    position: relative;
    flex-shrink: 0;
    width: var(--box);
    height: var(--box);
    margin-top: var(--outline);
  }

  input {
    position: absolute;
    inset: 0;
    z-index: 1;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: inherit;
  }

  .box {
    position: absolute;
    inset: 0;
    border: var(--switch-border) solid var(--color-border-strong);
    border-radius: var(--radius-small);
    transition-property: background-color, border-color;
    transition-duration: var(--duration-fast);
    transition-timing-function: var(--ease);
  }

  svg {
    position: absolute;
    top: calc(-1 * var(--switch-border));
    left: calc(-1 * var(--switch-border));
    max-width: none;
    height: var(--box);
  }

  path {
    display: none;
    stroke: var(--color-paper);
    stroke-width: var(--switch-border);
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  input:checked + .box,
  input:indeterminate + .box {
    background: var(--color-ink);
    border-color: var(--color-ink);
  }

  input:checked:not(:indeterminate) + .box .tick,
  input:indeterminate + .box .dash {
    display: inline;
  }

  input:focus-visible + .box {
    outline: var(--focus-ring) solid var(--color-accent);
    outline-offset: 2px;
  }

  .words {
    display: flex;
    flex-direction: column;
    gap: var(--segment-inset);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .strong {
    font-weight: var(--text-md-weight-strong);
  }

  .hint {
    color: var(--color-text-2);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }

  .waffle-checkbox:has(input:disabled) {
    opacity: 0.4;
    cursor: default;
  }

  /* Fills are dropped in forced colours: the mark takes the text colour. */
  @media (forced-colors: active) {
    path {
      stroke: currentColor;
    }
  }
</style>
