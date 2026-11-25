<script lang="ts">
  import { tick } from "svelte";
  import { firstRunLine, providerOrder, unsupportedLine } from "../../copy";
  import type { ProviderView } from "../../ipc/bindings/ProviderView";
  import { announce } from "../../stores/announce.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { usage } from "../../stores/usage.svelte";

  const known = $derived.by(() => {
    const found: ProviderView[] = [];
    for (const key of providerOrder) {
      const view = usage.snapshot?.providers.find((p) => p.provider === key);
      if (view !== undefined && !settings.value.hidden_providers.includes(key))
        found.push(view);
    }
    return found;
  });
  const lines = $derived(
    known
      .filter((p) => p.usage.state !== "unsupported")
      .map((p) => ({ key: p.provider, ...firstRunLine(p.provider, p.usage) })),
  );
  const placeholders = $derived(
    known.filter((p) => p.usage.state === "unsupported").map((p) => p.provider),
  );

  async function startAtLogin(box: HTMLInputElement): Promise<void> {
    const result = await settings.update({ autostart: box.checked });
    if (result.status === "error") {
      box.checked = settings.value.autostart;
      announce.say("Couldn't change the start at login setting");
    }
  }

  async function proceed(): Promise<void> {
    const result = await settings.update({ first_run_seen: true });
    if (result.status === "error") {
      announce.say("Couldn't save that. Try again");
      return;
    }
    await tick();
    document.getElementById("view-title")?.focus();
  }
</script>

<section aria-label="Welcome">
  <h1 id="view-title" tabindex="-1">Usage, in one place.</h1>
  <p>Claude and Codex appear automatically when they're set up.</p>
  <ul>
    {#each lines as line (line.key)}
      <li><span aria-hidden="true">{line.glyph}</span> {line.text}</li>
    {/each}
  </ul>
  {#if placeholders.length > 0}
    <p>{unsupportedLine(placeholders)}</p>
  {/if}
  <p class="start">
    Flashboard starts with your session and waits in the tray.
  </p>
  <label class="switch">
    <input
      type="checkbox"
      checked={settings.value.autostart}
      onchange={(event) => startAtLogin(event.currentTarget)}
    />
    Start at login
  </label>
  <button type="button" class="continue" onclick={proceed}>Continue</button>
</section>

<style>
  section {
    padding: var(--space-8) var(--space-4);
    max-width: 40rem;
    margin-inline: auto;
    color: var(--color-text);
    font-family: var(--font-sans);
  }

  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
    font-weight: var(--title-xl-weight);
    margin-bottom: var(--space-4);
  }

  ul {
    list-style: none;
    margin: var(--space-6) 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
    font-size: var(--text-md);
  }

  li span {
    display: inline-block;
    width: 1.25rem;
  }

  .start {
    margin-top: var(--space-6);
  }

  .switch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--hit-min);
    margin-top: var(--space-2);
  }

  .continue {
    margin-top: var(--space-8);
  }
</style>
