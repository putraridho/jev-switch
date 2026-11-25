<!-- A square icon button (S33): 32 or 28, a 16 px glyph with a 1.5 stroke. The label is
     required: it is the accessible name. -->
<script lang="ts">
  import type { IconName } from "./display";

  interface Props {
    label: string;
    icon: IconName;
    size?: "regular" | "small";
    /** Without the outline. */
    quiet?: boolean;
    disabled?: boolean;
    onclick: () => void;
  }

  const {
    label,
    icon,
    size = "regular",
    quiet = false,
    disabled = false,
    onclick,
  }: Props = $props();

  const paths: Record<IconName, string> = {
    refresh: "M13.25 8a5.25 5.25 0 1 1-1.55-3.7 M12 1.75V4.5H9.25",
    close: "M3.5 3.5l9 9 M12.5 3.5l-9 9",
    back: "M10 4 6 8l4 4",
    more: "M3.5 8h.01 M8 8h.01 M12.5 8h.01",
  };
</script>

<button
  type="button"
  class="icon"
  class:small={size === "small"}
  class:text={quiet}
  aria-label={label}
  title={label}
  {disabled}
  {onclick}
>
  <svg
    viewBox="0 0 16 16"
    width="16"
    height="16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d={paths[icon]} />
  </svg>
</button>

<style>
  .icon {
    flex-shrink: 0;
    width: var(--control-h);
    padding: 0;
  }

  .small {
    width: var(--control-sm-h);
  }

  svg {
    width: var(--icon-md);
    height: var(--icon-md);
  }
</style>
