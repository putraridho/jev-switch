<script lang="ts">
  // One provider's detail (docs/dashboard/DESIGN.md 9.4). T10 writes it.
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";

  type Props = { provider: ProviderKey };
  const { provider }: Props = $props();
</script>

<section aria-label="Provider detail" data-provider={provider}></section>
