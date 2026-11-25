<!-- The range switch (S24): a pill group over native radios, so Tab enters it once and the
     arrow keys move the choice. -->
<script lang="ts" generics="Value extends string">
  interface Props {
    /** Names the group; not drawn. */
    legend: string;
    /** `count` is drawn after the label (S36 "with counts"). */
    options: readonly { value: Value; label: string; count?: number }[];
    value: Value;
    disabled?: boolean;
    /** S36 segmented control: the pills sit in a ruled track. */
    track?: boolean;
    onchange: (value: Value) => void;
  }

  const {
    legend,
    options,
    value,
    disabled = false,
    track = false,
    onchange,
  }: Props = $props();

  const uid = $props.id();
</script>

<fieldset {disabled} class:track>
  <legend>{legend}</legend>
  {#each options as option (option.value)}
    <label class:on={value === option.value}>
      <input
        type="radio"
        name="{uid}-pills"
        value={option.value}
        checked={value === option.value}
        onchange={() => {
          onchange(option.value);
        }}
      />
      <span
        >{option.count === undefined
          ? option.label
          : `${option.label} `}{#if option.count !== undefined}<span
            class="count">{option.count}</span
          >{/if}</span
      >
    </label>
  {/each}
</fieldset>

<style>
  fieldset {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }

  legend {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }

  /* The pill is 28 high; the label around it is the 32 px target. */
  label {
    position: relative;
    display: flex;
    align-items: center;
    min-height: var(--hit-min);
    cursor: pointer;
  }

  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: inherit;
  }

  input + span {
    gap: var(--space-2);
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: var(--pill-min-w);
    min-height: var(--pill-h);
    padding-inline: var(--space-3);
    color: var(--color-text-2);
    border-radius: var(--radius-pill);
    font-size: var(--text-sm);
    font-weight: var(--text-sm-weight-strong);
    line-height: var(--text-sm-line);
    pointer-events: none;
    transition-property: background-color, color;
    transition-duration: var(--duration-fast);
    transition-timing-function: var(--ease);
  }

  label:hover input:not(:checked, :disabled) + span {
    color: var(--color-text);
    background: var(--color-hover);
  }

  input:checked + span {
    color: var(--color-paper);
    background: var(--color-ink);
  }

  input:focus-visible + span {
    outline: var(--focus-ring) solid var(--color-accent);
    outline-offset: 2px;
  }

  .track {
    gap: 0;
    width: fit-content;
    max-width: 100%;
    padding: var(--segment-inset);
    border: var(--outline) solid var(--color-rule);
    border-radius: var(--segment-radius);
  }

  .count {
    opacity: 0.7;
  }

  .track label {
    min-height: var(--pill-h);
  }

  /* The target stays 32 high inside the 28 px segment. */
  .track input {
    inset: calc(-1 * var(--segment-inset)) 0;
    height: auto;
  }

  fieldset:disabled label {
    opacity: 0.4;
    cursor: default;
  }

  /* Fills are dropped in forced colours: the chosen pill gets a border. */
  @media (forced-colors: active) {
    input:checked + span {
      border: var(--focus-ring) solid currentColor;
    }
  }
</style>
