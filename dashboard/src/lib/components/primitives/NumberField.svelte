<!-- A labelled number field (S34): label above, one line of help or error below. -->
<script lang="ts">
  interface Props {
    /** The input's id; the error is `{id}-error`, the help `{id}-help`. */
    id: string;
    label: string;
    value: number;
    min?: number;
    max?: number;
    step?: number;
    help?: string;
    /** The sentence that says why the value was refused; `null` when it is fine. */
    error?: string | null;
    /** Shows the word "Saved" under the field. */
    saved?: boolean;
    disabled?: boolean;
    onchange: (input: HTMLInputElement) => void;
    onblur?: (input: HTMLInputElement) => void;
  }

  const {
    id,
    label,
    value,
    min,
    max,
    step,
    help,
    error = null,
    saved = false,
    disabled = false,
    onchange,
    onblur,
  }: Props = $props();

  const describedBy = $derived(
    error !== null
      ? `${id}-error`
      : help !== undefined
        ? `${id}-help`
        : undefined,
  );
</script>

<div class="field">
  <label for={id}>{label}</label>
  <input
    {id}
    type="number"
    {value}
    {min}
    {max}
    {step}
    {disabled}
    aria-invalid={error !== null}
    aria-describedby={describedBy}
    onchange={(event) => {
      onchange(event.currentTarget);
    }}
    onblur={(event) => {
      onblur?.(event.currentTarget);
    }}
  />
  {#if error !== null}
    <p class="error" id="{id}-error">{error}</p>
  {:else if saved}
    <p class="saved">Saved</p>
  {:else if help !== undefined}
    <p id="{id}-help">{help}</p>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  label {
    margin-bottom: var(--space-2);
    font-size: var(--text-sm);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-sm-line);
  }

  p {
    margin-top: var(--space-1);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }

  .error {
    color: var(--color-alert);
  }
</style>
