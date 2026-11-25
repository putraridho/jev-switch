<script lang="ts">
  import StatusBadge from "./StatusBadge.svelte";

  interface Props {
    /** How old the last good value is: "Last good read 1m ago". */
    text: string;
  }

  const { text }: Props = $props();
</script>

<p><StatusBadge kind="stale" label={text} /></p>

<style>
  p {
    display: flex;
    margin: 0;
    min-height: var(--text-sm-line);
  }
</style>
