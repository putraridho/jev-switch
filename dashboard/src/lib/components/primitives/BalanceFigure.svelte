<script lang="ts">
  import BigNumber from "./BigNumber.svelte";

  interface Props {
    /** The formatted amount: "$18.40". */
    amount: string;
    /** "credits remaining", "Available". */
    caption: string;
    /** The balance is low or unusable: the figure turns alert. */
    low?: boolean;
  }

  const { amount, caption, low = false }: Props = $props();
</script>

<!-- S32: a balance has no limit, so no waffle. A missing balance is an ErrorState, never 0. -->
<div class="balance">
  <BigNumber value={amount} alert={low} />
  <p>{caption}</p>
</div>

<style>
  .balance {
    display: grid;
    gap: var(--space-1);
  }
  p {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    color: var(--color-muted);
  }
</style>
