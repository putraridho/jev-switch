<!-- The Waffle switch (S35, rows as in S53): a native checkbox with `role="switch"`. The
     whole row is the label, so the whole row toggles it. -->
<script lang="ts">
  interface Props {
    /** The accessible name: the visible label. */
    label: string;
    checked: boolean;
    /** One muted line about the setting; it is the switch's description. */
    hint?: string;
    disabled?: boolean;
    /** The label in 600, for a row that names a thing (a provider) rather than a sentence. */
    strong?: boolean;
    /** Where the switch sits: after the label (settings rows) or before it. */
    placement?: "end" | "start";
    onchange: (checked: boolean, input: HTMLInputElement) => void;
    /** The input, for a caller that moves focus back or reverts the state. */
    element?: HTMLInputElement | null;
  }

  let {
    label,
    checked,
    hint,
    disabled = false,
    strong = false,
    placement = "end",
    onchange,
    element = $bindable(null),
  }: Props = $props();

  const uid = $props.id();
</script>

<label class="waffle-switch" class:start={placement === "start"}>
  <span class="words">
    <span class="name" class:strong id="{uid}-name">{label}</span>
    {#if hint !== undefined}<span class="hint" id="{uid}-hint">{hint}</span
      >{/if}
  </span>
  <span class="control">
    <input
      bind:this={element}
      type="checkbox"
      role="switch"
      {checked}
      {disabled}
      aria-labelledby="{uid}-name"
      aria-describedby={hint === undefined ? undefined : `${uid}-hint`}
      onchange={(event) => {
        onchange(event.currentTarget.checked, event.currentTarget);
      }}
    />
    <span class="track" aria-hidden="true"><span class="knob"></span></span>
  </span>
</label>

<style>
  .waffle-switch {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: var(--row-h);
    padding-block: var(--space-2);
    cursor: pointer;
  }

  .start {
    flex-direction: row-reverse;
    justify-content: flex-end;
  }

  .words {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    column-gap: var(--space-4);
    min-width: 0;
  }

  .name {
    font-size: var(--text-md);
    line-height: var(--text-md-line);
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

  .control {
    position: relative;
    flex-shrink: 0;
    width: var(--switch-w);
    height: var(--switch-h);
  }

  /* The real control: invisible, but it keeps focus, keys and pointer. */
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

  .track {
    position: absolute;
    inset: 0;
    border: var(--switch-border) solid var(--color-border-strong);
    border-radius: var(--radius-switch);
    transition-property: background-color, border-color;
    transition-duration: var(--duration-fast);
    transition-timing-function: var(--ease);
  }

  /* The knob is drawn with its border, so it survives forced colours. Positions are measured
     inside the track's own border. */
  .knob {
    position: absolute;
    top: calc(
      (var(--switch-h) - var(--switch-knob-off)) / 2 - var(--switch-border)
    );
    left: calc(var(--switch-inset) - var(--switch-border));
    width: var(--switch-knob-off);
    height: var(--switch-knob-off);
    border: calc(var(--switch-knob-off) / 2) solid var(--color-border-strong);
    border-radius: var(--radius-full);
    transition-property: left, top, width, height, border-width, border-color;
    transition-duration: var(--duration-fast);
    transition-timing-function: var(--ease);
  }

  input:checked + .track {
    background: var(--color-ink);
    border-color: var(--color-ink);
  }

  input:checked + .track .knob {
    top: calc(var(--switch-inset) - var(--switch-border));
    left: calc(
      var(--switch-w) - var(--switch-knob-on) - var(--switch-inset) -
        var(--switch-border)
    );
    width: var(--switch-knob-on);
    height: var(--switch-knob-on);
    border-width: calc(var(--switch-knob-on) / 2);
    border-color: var(--color-paper);
  }

  input:focus-visible + .track {
    outline: var(--focus-ring) solid var(--color-accent);
    outline-offset: 2px;
  }

  .waffle-switch:has(input:disabled) {
    opacity: 0.4;
    cursor: default;
  }

  @media (forced-colors: active) {
    .track,
    input:checked + .track .knob,
    .knob {
      border-color: currentColor;
    }

    input:checked + .track {
      border-width: var(--switch-inset);
    }

    input:checked + .track .knob {
      top: 0;
      left: calc(
        var(--switch-w) - var(--switch-knob-on) - 2 * var(--switch-inset)
      );
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .knob {
      transition-property: border-color;
    }
  }
</style>
