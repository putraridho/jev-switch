<script lang="ts">
  import { tick } from "svelte";
  import DiagnosticsPanel from "../lib/components/shell/DiagnosticsPanel.svelte";
  import { pushEscape } from "../lib/escape";
  import {
    checkCritical,
    checkWarn,
    type ThresholdCheck,
  } from "../lib/thresholds";
  import { settings } from "../lib/stores/settings.svelte";
  import { announce } from "../lib/stores/announce.svelte";
  import type { SettingsPatch } from "../lib/ipc/bindings/SettingsPatch";
  import { providerName, providerOrder } from "../lib/copy";
  import { behaviourId } from "../lib/shortcuts";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  const {}: Record<string, string> = $props();
  const providers: readonly ProviderKey[] = providerOrder;
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
  async function update(patch: SettingsPatch): Promise<boolean> {
    const result = await settings.update(patch);
    if (result.status === "error") {
      announce.say("Could not save settings");
      return false;
    }
    announce.say("Saved");
    return true;
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

  // Turning on the Claude live check asks first: it sends one small request per check.
  let confirming = $state(false);
  let dialog = $state<HTMLDialogElement | null>(null);
  let liveBox = $state<HTMLInputElement | null>(null);

  async function closeConfirm(): Promise<void> {
    confirming = false;
    if (liveBox) liveBox.checked = settings.value.claude_live_check;
    await tick();
    liveBox?.focus();
  }

  $effect(() => {
    if (!confirming || dialog === null) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
    return pushEscape("dialog", () => {
      void closeConfirm();
      return true;
    });
  });
  function hide(provider: ProviderKey, hidden: boolean) {
    const existing = settings.value.hidden_providers.filter(
      (item) => item !== provider,
    );
    void update({
      hidden_providers: hidden ? [...existing, provider] : existing,
    });
  }
</script>

<section aria-label="Settings">
  <h1 id="view-title" tabindex="-1">Settings</h1>
  <h2>Thresholds</h2>
  <div class="threshold">
    <label class="field"
      >Warning (%) <input
        type="number"
        min="50"
        max="100"
        value={settings.value.warn_bp / 100}
        aria-invalid={warnError !== null}
        aria-describedby={warnError === null ? undefined : "warn-error"}
        onchange={(event) => commit("warn", event.currentTarget)}
        onblur={(event) => {
          restore("warn", event.currentTarget);
        }}
      /></label
    >
    {#if warnError !== null}<p class="error" id="warn-error">
        {warnError}
      </p>{/if}
    {#if saved === "warn"}<p class="saved">Saved</p>{/if}
  </div>
  <div class="threshold">
    <label class="field"
      >Critical (%) <input
        type="number"
        min="50"
        max="100"
        value={settings.value.critical_bp / 100}
        aria-invalid={criticalError !== null}
        aria-describedby={criticalError === null ? undefined : "critical-error"}
        onchange={(event) => commit("critical", event.currentTarget)}
        onblur={(event) => {
          restore("critical", event.currentTarget);
        }}
      /></label
    >
    {#if criticalError !== null}<p class="error" id="critical-error">
        {criticalError}
      </p>{/if}
    {#if saved === "critical"}<p class="saved">Saved</p>{/if}
  </div>
  <h2>Notifications</h2>
  <label
    ><input
      type="checkbox"
      checked={settings.value.notifications}
      onchange={(event) =>
        update({ notifications: event.currentTarget.checked })}
    /> Enable notifications</label
  >
  <label
    ><input
      type="checkbox"
      checked={settings.value.notify_warn}
      onchange={(event) => update({ notify_warn: event.currentTarget.checked })}
    /> Warning notifications</label
  >
  <label
    ><input
      type="checkbox"
      checked={settings.value.notify_critical}
      onchange={(event) =>
        update({ notify_critical: event.currentTarget.checked })}
    /> Critical notifications</label
  >
  <h2>Appearance</h2>
  <label class="field"
    >Theme <select
      value={settings.value.theme}
      onchange={(event) => {
        const theme = event.currentTarget.value;
        if (theme === "system" || theme === "light" || theme === "dark")
          void update({ theme });
      }}
      ><option value="system">System</option><option value="light">Light</option
      ><option value="dark">Dark</option></select
    ></label
  >
  <label class="field"
    >Zoom (%) <input
      type="number"
      min="100"
      max="200"
      step="25"
      value={settings.value.zoom_pct}
      onchange={(event) =>
        update({ zoom_pct: event.currentTarget.valueAsNumber })}
    /></label
  >
  <h2>Startup</h2>
  <label
    ><input
      type="checkbox"
      checked={settings.value.autostart}
      onchange={(event) => update({ autostart: event.currentTarget.checked })}
    /> Start automatically</label
  >
  <h2>Hidden providers</h2>
  {#each providers as provider (provider)}<label
      ><input
        type="checkbox"
        checked={settings.value.hidden_providers.includes(provider)}
        onchange={(event) => {
          hide(provider, event.currentTarget.checked);
        }}
      />
      Hide {providerName[provider]}</label
    >{/each}
  <label
    ><input
      bind:this={liveBox}
      type="checkbox"
      checked={settings.value.claude_live_check}
      onchange={(event) => {
        if (event.currentTarget.checked) confirming = true;
        else void update({ claude_live_check: false });
      }}
    /> Claude live check · uses one small request per check</label
  >
  {#if confirming}
    <dialog
      bind:this={dialog}
      aria-labelledby="live-title"
      oncancel={(event) => {
        event.preventDefault();
      }}
    >
      <h2 id="live-title">Turn on the Claude live check?</h2>
      <p>
        Flashboard will send one small request to Claude each time it checks. It
        uses a little of your plan.
      </p>
      <div class="actions">
        <button type="button" onclick={closeConfirm}>Cancel</button>
        <button
          type="button"
          onclick={async () => {
            const ok = await update({ claude_live_check: true });
            if (ok) {
              confirming = false;
              await tick();
              liveBox?.focus();
            } else await closeConfirm();
          }}>Turn on</button
        >
      </div>
    </dialog>
  {/if}
  <h2 id={behaviourId} tabindex="-1">Behaviour</h2>
  <label
    ><input
      type="checkbox"
      checked={settings.value.shortcuts}
      onchange={(event) => update({ shortcuts: event.currentTarget.checked })}
    /> Single-key shortcuts</label
  >
  <dl>
    {#each keys as key (key.keys)}
      <div>
        <dt><kbd>{key.keys}</kbd></dt>
        <dd>{key.does}</dd>
      </div>
    {/each}
  </dl>
  <DiagnosticsPanel />
</section>

<style>
  section {
    padding: var(--space-6) var(--space-4);
    max-width: 40rem;
    margin-inline: auto;
    color: var(--color-text);
    font-family: var(--font-sans);
  }
  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
  }
  h2 {
    margin-top: var(--space-8);
    font-size: var(--text-md);
  }
  label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--hit-min);
  }
  .field {
    display: grid;
    grid-template-columns: 9rem max-content;
  }
  .error {
    margin: 0 0 var(--space-2);
    color: var(--color-alert);
    font-size: var(--text-sm);
  }
  .saved {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
  }
  dialog {
    max-width: 24rem;
    padding: var(--space-6);
    color: var(--color-text);
    background: var(--color-paper);
    border: var(--outline) solid var(--color-border-strong);
    border-radius: var(--radius-control);
  }
  dialog h2 {
    margin-top: 0;
  }
  .actions {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-4);
  }
  input,
  select {
    font: inherit;
  }
  dl {
    display: grid;
    gap: var(--space-1);
    margin: var(--space-2) 0 0;
  }
  dl div {
    display: grid;
    grid-template-columns: 8rem 1fr;
    gap: var(--space-4);
  }
  dt,
  dd {
    margin: 0;
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }
  dd {
    color: var(--color-text-2);
  }
  kbd {
    font-family: var(--font-mono);
  }
</style>
