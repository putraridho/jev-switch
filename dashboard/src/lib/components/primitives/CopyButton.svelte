<script lang="ts">
  import { copyText } from "../../actions";

  interface Props {
    /** What goes to the clipboard: a command or a variable name. */
    text: string;
  }

  const { text }: Props = $props();

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function copy(): Promise<void> {
    copied = await copyText(text);
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      copied = false;
    }, 2_000);
  }
</script>

<!-- The visible word is "Copy"; the name starts with it and says what is copied (WCAG 2.5.3). -->
<button
  type="button"
  aria-label="{copied ? 'Copied' : 'Copy'} {text}"
  onclick={copy}
>
  <svg
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {#if copied}
      <path d="M3 8.5 L6.5 12 L13 4.5" />
    {:else}
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path
        d="M10.5 5.5V4A1.5 1.5 0 0 0 9 2.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5"
      />
    {/if}
  </svg>
  {copied ? "Copied" : "Copy"}
</button>

<style>
  button {
    flex-shrink: 0;
    gap: var(--space-2);
    padding-inline: var(--space-3);
  }

  svg {
    width: var(--icon-sm);
    height: var(--icon-sm);
  }
</style>
