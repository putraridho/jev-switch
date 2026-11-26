<!-- First run (DESIGN 9.5), drawn as the Waffle "connect a provider" view (S54): what was
     found as hairline rows, the one command where a provider needs one, the placeholders
     named under "Not supported yet", then Start at login and Continue. -->
<script lang="ts">
  import { tick } from "svelte";
  import {
    firstRunLine,
    providerHint,
    providerOrder,
    setupCopy,
    unavailableCopy,
    unsupportedLine,
  } from "../../copy";
  import type { ProviderKey } from "../../ipc/bindings/ProviderKey";
  import type { ProviderView } from "../../ipc/bindings/ProviderView";
  import type { UsageView } from "../../ipc/bindings/UsageView";
  import { announce } from "../../stores/announce.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { usage } from "../../stores/usage.svelte";
  import CommandLine from "../primitives/CommandLine.svelte";
  import Section from "../primitives/Section.svelte";
  import Switch from "../primitives/Switch.svelte";

  const known = $derived.by(() => {
    const found: ProviderView[] = [];
    for (const key of providerOrder) {
      const view = usage.snapshot?.providers.find((p) => p.provider === key);
      if (view !== undefined && !settings.value.hidden_providers.includes(key))
        found.push(view);
    }
    return found;
  });

  /** The command that would connect a provider, when there is one to copy. */
  function command(provider: ProviderKey, view: UsageView): string | undefined {
    if (view.state === "not_configured")
      return setupCopy(view.setup, provider).command;
    if (view.state === "unavailable")
      return unavailableCopy(view.reason, provider).command;
    return undefined;
  }

  const lines = $derived(
    known
      .filter((p) => p.usage.state !== "unsupported")
      .map((p) => ({
        key: p.provider,
        command: command(p.provider, p.usage),
        ...firstRunLine(p.provider, p.usage),
      })),
  );
  const placeholders = $derived(
    known.filter((p) => p.usage.state === "unsupported").map((p) => p.provider),
  );

  async function startAtLogin(
    on: boolean,
    input: HTMLInputElement,
  ): Promise<void> {
    const result = await settings.update({ autostart: on });
    if (result.status === "error") {
      input.checked = settings.value.autostart;
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

<section class="page" aria-label="Welcome">
  <header>
    <h1 id="view-title" tabindex="-1">Usage, in one place.</h1>
    <p>Claude and Codex appear automatically when they're set up.</p>
  </header>

  <Section label="Providers">
    <ul>
      {#each lines as line (line.key)}
        <li>
          <span class="glyph" aria-hidden="true">{line.glyph}</span>
          <div class="words">
            <span class="name">{line.text}</span>
            <span class="hint">{providerHint[line.key]}</span>
            {#if line.command !== undefined}
              <CommandLine command={line.command} />
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  </Section>

  {#if placeholders.length > 0}
    <Section label="Not supported yet">
      <p>{unsupportedLine(placeholders)}</p>
    </Section>
  {/if}

  <Section>
    <p class="tray">
      Flashboard starts with your session and waits in the tray.
    </p>
    <Switch
      label="Start at login"
      checked={settings.value.autostart}
      onchange={startAtLogin}
    />
    <div class="actions">
      <button type="button" class="primary" onclick={proceed}>Continue</button>
    </div>
  </Section>
</section>

<style>
  .page {
    padding: var(--space-6) var(--space-4) var(--space-12);
    max-width: 40rem;
    margin-inline: auto;
    color: var(--color-text);
    font-family: var(--font-sans);
  }

  header {
    display: grid;
    gap: var(--space-2);
  }

  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
    font-weight: var(--title-xl-weight);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: grid;
    grid-template-columns: var(--space-6) minmax(0, 1fr);
    padding-block: var(--space-2);
  }

  li + li {
    border-top: var(--rule) solid var(--color-rule);
  }

  .glyph {
    font-size: var(--text-sm);
    line-height: var(--text-md-line);
  }

  .words {
    display: grid;
    justify-items: start;
    gap: var(--space-1);
    min-width: 0;
  }

  .name {
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-md-line);
  }

  .hint {
    color: var(--color-text-2);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }

  .tray {
    margin-bottom: var(--space-2);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-6);
  }
</style>
