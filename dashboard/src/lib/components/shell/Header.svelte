<script lang="ts">
  import { router } from "../../router.svelte";
</script>

<header>
  <button
    type="button"
    onclick={() => {
      router.go({ name: "overview" });
    }}>Flashboard</button
  >
  <button
    type="button"
    onclick={() => {
      router.go({ name: "settings" });
    }}>Settings</button
  >
</header>
