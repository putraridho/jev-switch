<!-- Radios drawn as the waffle dot (S35): empty is the meter's outline cell, selected is a
     filled dot. Native radios, so Tab enters the group once and arrows move. -->
<script lang="ts" generics="Value extends string">
  interface Props {
    legend: string;
    options: readonly { value: Value; label: string }[];
    value: Value;
    disabled?: boolean;
    onchange: (value: Value) => void;
  }

  const {
    legend,
    options,
    value,
    disabled = false,
    onchange,
  }: Props = $props();

  const uid = $props.id();
</script>

<fieldset {disabled}>
  <legend>{legend}</legend>
  {#each options as option (option.value)}
    <label>
      <span class="control">
        <input
          type="radio"
          name="{uid}-radio"
          value={option.value}
          checked={value === option.value}
          onchange={() => {
            onchange(option.value);
          }}
        />
        <svg viewBox="0 0 18 18" width="18" height="18" aria-hidden="true">
          <circle class="empty" cx="9" cy="9" r="7.25" />
          <circle class="full" cx="9" cy="9" r="8" />
        </svg>
      </span>
      <span>{option.label}</span>
    </label>
  {/each}
</fieldset>

<style>
  fieldset {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }

  legend {
    padding: 0;
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-md-line);
  }

  /* 32 px rows: 16 px between dots' centres edges, the D9.10 target. */
  label {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--hit-min);
    cursor: pointer;
  }

  .control {
    position: relative;
    flex-shrink: 0;
    width: var(--box);
    height: var(--box);
    border-radius: var(--radius-full);
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

  svg {
    display: block;
    max-width: none;
    height: var(--box);
  }

  .empty {
    fill: none;
    stroke: var(--color-faint);
    stroke-width: var(--waffle-empty-stroke);
  }

  .full {
    display: none;
    fill: var(--color-ink);
  }

  input:checked + svg .full {
    display: inline;
  }

  .control:has(input:focus-visible) {
    outline: var(--focus-ring) solid var(--color-accent);
    outline-offset: 2px;
  }

  fieldset:disabled label {
    opacity: 0.4;
    cursor: default;
  }

  @media (forced-colors: active) {
    .empty {
      stroke: currentColor;
    }

    .full {
      fill: currentColor;
    }
  }
</style>
