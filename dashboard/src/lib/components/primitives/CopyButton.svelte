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
  {copied ? "Copied" : "Copy"}
</button>
