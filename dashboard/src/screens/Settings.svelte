<!-- Settings (Waffle S53, DESIGN 9.6): hairline sections, one row per setting, the label on
     the left and a switch on the right. Everything applies at once. -->
<script lang="ts">
  import { tick } from "svelte";
  import ConfirmDialog from "../lib/components/primitives/ConfirmDialog.svelte";
  import NumberField from "../lib/components/primitives/NumberField.svelte";
  import Section from "../lib/components/primitives/Section.svelte";
  import SelectField from "../lib/components/primitives/SelectField.svelte";
  import Switch from "../lib/components/primitives/Switch.svelte";
  import DiagnosticsPanel from "../lib/components/shell/DiagnosticsPanel.svelte";
  import { providerHint, providerName, providerOrder } from "../lib/copy";
  import type { HourCycle } from "../lib/ipc/bindings/HourCycle";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  import type { SettingsPatch } from "../lib/ipc/bindings/SettingsPatch";
  import type { Theme } from "../lib/ipc/bindings/Theme";
  import { behaviourId } from "../lib/shortcuts";
  import { announce } from "../lib/stores/announce.svelte";
  import { queries } from "../lib/stores/queries";
  import { settings } from "../lib/stores/settings.svelte";
  import {
    checkCritical,
    checkWarn,
    type ThresholdCheck,
  } from "../lib/thresholds";

  const {}: Record<string, string> = $props();

  const keys: readonly { keys: string; does: string }[] = [
    { keys: "R", does: "Refresh every provider" },
    { keys: ",", does: "Open Settings" },
    { keys: "?", does: "Open this list" },
    {
      keys: "1 to 6",
      does: "Open Claude, Codex, DeepSeek, Gemini, Antigravity, Grok",
    },
    { keys: "Esc", does: "Close, else go back" },
    { keys: "Ctrl and +", does: "Zoom in" },
    { keys: "Ctrl and -", does: "Zoom out" },
    { keys: "Ctrl and 0", does: "Reset zoom" },
  ];

  const themes: readonly { value: Theme; label: string }[] = [
    { value: "system", label: "System" },
    { value: "light", label: "Light" },
    { value: "dark", label: "Dark" },
  ];

  const clocks: readonly { value: HourCycle; label: string }[] = [
    { value: "auto", label: "Auto" },
    { value: "h12", label: "12-hour" },
    { value: "h24", label: "24-hour" },
  ];

  async function update(patch: SettingsPatch): Promise<boolean> {
    const result = await settings.update(patch);
    if (result.status === "error") {
      announce.say("Could not save settings");
      return false;
    }
    announce.say("Saved");
    return true;
  }

  // A switch that could not be saved goes back to what is stored (DESIGN 9.6).
  async function toggle(
    patch: SettingsPatch,
    input: HTMLInputElement,
  ): Promise<void> {
    const wanted = input.checked;
    if (!(await update(patch))) input.checked = !wanted;
  }

  // The two threshold fields: a value that is not allowed is not sent. The field goes back
  // to the saved value and the sentence next to it says why (WCAG 3.3.1, 3.3.3).
  let warnError = $state<string | null>(null);
  let criticalError = $state<string | null>(null);
  let saved = $state<"warn" | "critical" | null>(null);

  async function commit(
    field: "warn" | "critical",
    input: HTMLInputElement,
  ): Promise<void> {
    const percent = input.valueAsNumber;
    const check: ThresholdCheck =
      field === "warn"
        ? checkWarn(percent, settings.value.critical_bp)
        : checkCritical(percent, settings.value.warn_bp);
    const set = (message: string | null) => {
      if (field === "warn") warnError = message;
      else criticalError = message;
    };
    if (!check.ok) {
      set(check.message);
      saved = null;
      return;
    }
    set(null);
    const ok = await update(
      field === "warn" ? { warn_bp: check.bp } : { critical_bp: check.bp },
    );
    saved = ok ? field : null;
  }

  // On blur a field that was refused goes back to the saved value; the sentence stays.
  function restore(field: "warn" | "critical", input: HTMLInputElement): void {
    if ((field === "warn" ? warnError : criticalError) === null) return;
    input.value = String(
      (field === "warn" ? settings.value.warn_bp : settings.value.critical_bp) /
        100,
    );
  }

  // Without a tray there is nothing to keep running in (DESIGN 9.6).
  let trayMissing = $state(false);
  $effect(() => {
    void queries.diagnostics().then((result) => {
      trayMissing = result.status === "ok" && !result.value.tray_available;
    });
  });

  // Turning on the Claude live check asks first: it sends one small request per check.
  let confirming = $state(false);
  let liveSwitch = $state<HTMLInputElement | null>(null);

  async function closeConfirm(): Promise<void> {
    confirming = false;
    if (liveSwitch) liveSwitch.checked = settings.value.claude_live_check;
    await tick();
    liveSwitch?.focus();
  }

  async function turnOnLiveCheck(): Promise<void> {
    await update({ claude_live_check: true });
    await closeConfirm();
  }

  function show(provider: ProviderKey, shown: boolean): SettingsPatch {
    const others = settings.value.hidden_providers.filter(
      (item) => item !== provider,
    );
    return { hidden_providers: shown ? others : [...others, provider] };
  }
</script>

<section class="page" aria-label="Settings">
  <header>
    <h1 id="view-title" tabindex="-1">Settings</h1>
    <p>Choose which providers Flashboard shows and where the marks sit.</p>
  </header>

  <Section label="Providers">
    <div class="rows providers">
      {#each providerOrder as provider (provider)}
        <Switch
          label={providerName[provider]}
          hint={providerHint[provider]}
          strong
          checked={!settings.value.hidden_providers.includes(provider)}
          onchange={(shown: boolean, input: HTMLInputElement) =>
            toggle(show(provider, shown), input)}
        />
      {/each}
      <Switch
        label="Claude live check"
        hint="Uses one small request per check."
        checked={settings.value.claude_live_check}
        bind:element={liveSwitch}
        onchange={(on: boolean, input: HTMLInputElement) => {
          if (on) confirming = true;
          else void toggle({ claude_live_check: false }, input);
        }}
      />
    </div>
  </Section>

  <Section label="Thresholds">
    <div class="fields">
      <NumberField
        id="warn"
        label="Warning (%)"
        min={50}
        max={100}
        value={settings.value.warn_bp / 100}
        error={warnError}
        saved={saved === "warn"}
        onchange={(input: HTMLInputElement) => commit("warn", input)}
        onblur={(input: HTMLInputElement) => {
          restore("warn", input);
        }}
      />
      <NumberField
        id="critical"
        label="Critical (%)"
        min={50}
        max={100}
        value={settings.value.critical_bp / 100}
        error={criticalError}
        saved={saved === "critical"}
        onchange={(input: HTMLInputElement) => commit("critical", input)}
        onblur={(input: HTMLInputElement) => {
          restore("critical", input);
        }}
      />
    </div>
    <p class="help">
      Meters turn red past the warning mark. Critical must stay above warning.
    </p>
  </Section>

  <Section label="Notifications">
    <div class="rows">
      <Switch
        label="Send notifications"
        hint="Once per window, never repeated."
        checked={settings.value.notifications}
        onchange={(on: boolean, input: HTMLInputElement) =>
          toggle({ notifications: on }, input)}
      />
      <Switch
        label="Tell me when a provider passes the warning mark"
        checked={settings.value.notify_warn}
        disabled={!settings.value.notifications}
        onchange={(on: boolean, input: HTMLInputElement) =>
          toggle({ notify_warn: on }, input)}
      />
      <Switch
        label="Tell me when a provider passes the critical mark"
        checked={settings.value.notify_critical}
        disabled={!settings.value.notifications}
        onchange={(on: boolean, input: HTMLInputElement) =>
          toggle({ notify_critical: on }, input)}
      />
    </div>
  </Section>

  <Section label="Appearance">
    <div class="fields">
      <SelectField
        id="theme"
        label="Theme"
        value={settings.value.theme}
        options={themes}
        onchange={(theme: Theme) => void update({ theme })}
      />
      <SelectField
        id="clock"
        label="Clock"
        value={settings.value.hour_cycle}
        options={clocks}
        onchange={(hour_cycle: HourCycle) => void update({ hour_cycle })}
      />
      <NumberField
        id="zoom"
        label="Zoom (%)"
        min={100}
        max={200}
        step={25}
        value={settings.value.zoom_pct}
        onchange={(input: HTMLInputElement) =>
          void update({ zoom_pct: input.valueAsNumber })}
      />
    </div>
  </Section>

  <Section>
    <h2 id={behaviourId} tabindex="-1">Behaviour</h2>
    <div class="rows">
      <Switch
        label="Start at login"
        hint="Waits in the tray."
        checked={settings.value.autostart}
        onchange={(on: boolean, input: HTMLInputElement) =>
          toggle({ autostart: on }, input)}
      />
      <Switch
        label="Keep running in the tray when the window closes"
        {...trayMissing ? { hint: "No tray is available here." } : {}}
        checked={settings.value.close_to_tray}
        disabled={trayMissing}
        onchange={(on: boolean, input: HTMLInputElement) =>
          toggle({ close_to_tray: on }, input)}
      />
      <Switch
        label="Single-key shortcuts"
        checked={settings.value.shortcuts}
        onchange={(on: boolean, input: HTMLInputElement) =>
          toggle({ shortcuts: on }, input)}
      />
    </div>
    <dl>
      {#each keys as key (key.keys)}
        <div>
          <dt><kbd>{key.keys}</kbd></dt>
          <dd>{key.does}</dd>
        </div>
      {/each}
    </dl>
  </Section>

  <Section>
    <DiagnosticsPanel />
  </Section>

  {#if confirming}
    <ConfirmDialog
      title="Turn on the Claude live check?"
      body="Flashboard will send one small request to Claude each time it checks. It uses a little of your plan."
      confirmLabel="Turn on"
      onconfirm={() => void turnOnLiveCheck()}
      oncancel={() => void closeConfirm()}
    />
  {/if}
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

  h2 {
    margin: 0 0 var(--space-4);
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    font-weight: var(--text-md-weight-strong);
  }

  /* A description sits under its name; with room, S53 puts it beside, in one column. */
  .rows :global(.words) {
    flex-direction: column;
    align-items: flex-start;
  }

  @media (min-width: 34rem) {
    .providers :global(.words) {
      flex-direction: row;
      align-items: baseline;
    }

    .providers :global(.name.strong) {
      min-width: 7.5rem;
    }
  }

  .rows :global(.waffle-switch:first-child) {
    padding-block-start: 0;
    min-height: var(--control-h);
  }

  .fields {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-4) var(--space-6);
  }

  .fields :global(.field) {
    max-width: 11rem;
  }

  .help {
    margin-top: var(--space-3);
  }

  dl {
    margin: var(--space-4) 0 0;
  }

  dl div {
    display: grid;
    grid-template-columns: 7rem minmax(0, 1fr);
    gap: var(--space-4);
    padding-block: var(--space-1);
  }

  dt,
  dd {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }

  dd {
    color: var(--color-text-2);
    overflow-wrap: anywhere;
  }

  kbd {
    font-family: var(--font-mono);
  }
</style>
