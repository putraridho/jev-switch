<script lang="ts">
  // The app shell. T07 mounts the header, the router outlet with the three screens and
  // the single live region here; the scaffold shows the product name only.
  const title = "Flashboard";
</script>

<main>
  <h1>{title}</h1>
</main>

<style>
  main {
    min-height: 100vh;
    padding: var(--space-6);
    background: var(--color-bg);
    color: var(--color-text);
    font-family: var(--font-sans);
  }

  h1 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: var(--text-xl-weight);
    line-height: var(--text-xl-line);
  }
</style>
