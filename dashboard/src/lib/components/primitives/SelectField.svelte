<!-- A labelled native select, drawn like the text field (S34): label above. -->
<script lang="ts" generics="Value extends string">
  interface Props {
    id: string;
    label: string;
    value: Value | undefined;
    options: readonly { value: Value; label: string }[];
    disabled?: boolean;
    onchange: (value: Value) => void;
  }

  const {
    id,
    label,
    value,
    options,
    disabled = false,
    onchange,
  }: Props = $props();
</script>

<div class="field">
  <label for={id}>{label}</label>
  <select
    {id}
    {value}
    {disabled}
    onchange={(event) => {
      const chosen = options.find(
        (option) => option.value === event.currentTarget.value,
      );
      if (chosen !== undefined) onchange(chosen.value);
    }}
  >
    {#each options as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
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

  select {
    min-width: 7.5rem;
    max-width: 100%;
  }
</style>
