<script lang="ts">
  import { settings } from "../lib/stores/settings.svelte";
  import { announce } from "../lib/stores/announce.svelte";
  import type { SettingsPatch } from "../lib/ipc/bindings/SettingsPatch";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  const {}: Record<string, string> = $props();
  const providers: ProviderKey[] = [
    "claude",
    "codex",
    "deepseek",
    "gemini",
    "antigravity",
    "grok",
  ];
  async function update(patch: SettingsPatch) {
    const result = await settings.update(patch);
    if (result.status === "error") announce.say("Could not save settings");
  }
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
  <h1>Settings</h1>
  <h2>Thresholds</h2>
  <label
    >Warning (%) <input
      type="number"
      min="0"
      max="100"
      value={settings.value.warn_bp / 100}
      onchange={(event) =>
        update({ warn_bp: event.currentTarget.valueAsNumber * 100 })}
    /></label
  >
  <label
    >Critical (%) <input
      type="number"
      min="0"
      max="100"
      value={settings.value.critical_bp / 100}
      onchange={(event) =>
        update({ critical_bp: event.currentTarget.valueAsNumber * 100 })}
    /></label
  >
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
  <label
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
  <label
    >Zoom (%) <input
      type="number"
      min="50"
      max="200"
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
      Hide {provider}</label
    >{/each}
  <h2>Diagnostics</h2>
  <p>Diagnostics are not available on this screen yet.</p>
</section>

<style>
  section {
    padding: var(--space-12);
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
  input,
  select {
    font: inherit;
  }
</style>
