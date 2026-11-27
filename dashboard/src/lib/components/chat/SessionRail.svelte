<!-- The chat's right rail (C1): where the agents work, every provider's plan usage as a small
     waffle (so you can see what a turn will draw on before you send it), and what this
     session has changed. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Meter from "../primitives/Meter.svelte";
  import { planUsage } from "../../chat/quota";
  import { planName, providerName, providerOrder } from "../../copy";
  import { countdown, percent, windowName } from "../../format";
  import type { ProviderKey } from "../../ipc/bindings/ProviderKey";
  import type { Snapshot } from "../../ipc/bindings/Snapshot";
  import type { ChatProviderStatus } from "../../ipc/chat-bindings/ChatProviderStatus";
  import type { ChangedFile } from "../../chat/session";

  interface Props {
    workspace: string;
    folderLabel: string;
    folderDisabled?: boolean;
    onchangefolder: () => void;
    /** The folder picker, shown under the folder while it's open. */
    picker?: Snippet;
    providers: readonly ChatProviderStatus[];
    snapshot: Snapshot | null;
    now: number;
    /** The provider answering the turn in flight. */
    answering?: ProviderKey | null;
    changed: readonly ChangedFile[];
    /** "4,212 in · 612 out", or empty. */
    tokens?: string;
  }

  const {
    workspace,
    folderLabel,
    folderDisabled = false,
    onchangefolder,
    picker,
    providers,
    snapshot,
    now,
    answering = null,
    changed,
    tokens = "",
  }: Props = $props();

  const rows = $derived(
    providerOrder
      .flatMap((key) => {
        const status = providers.find((p) => p.provider === key);
        return status === undefined ? [] : [status];
      })
      .map((status) => {
        const usage = planUsage(snapshot, status.provider);
        const resets =
          usage?.meter.resets_at != null
            ? countdown(usage.meter.resets_at, now)
            : null;
        const parts: string[] = [];
        if (status.plan) parts.push(planName(status.plan));
        if (usage) parts.push(windowName(usage.meter.window));
        if (resets) parts.push(`Resets in ${resets}`);
        return {
          key: status.provider,
          ready: status.ready,
          usage,
          line: !status.ready
            ? "Not set up"
            : parts.length > 0
              ? parts.join(" · ")
              : status.detail,
        };
      }),
  );
</script>

<aside class="rail" aria-label="This session">
  <section class="block">
    <h2 class="label">Working in</h2>
    <button
      type="button"
      class="folder text"
      aria-label={folderLabel}
      disabled={folderDisabled}
      onclick={onchangefolder}
    >
      <span class="path">{workspace === "" ? "." : workspace}</span>
      <span class="change" aria-hidden="true">Change</span>
    </button>
    {#if picker}{@render picker()}{/if}
  </section>

  <section class="block">
    <h2 class="label">Providers</h2>
    <ul class="providers">
      {#each rows as row (row.key)}
        <li
          class="provider"
          class:answering={row.key === answering}
          class:off={!row.ready}
        >
          {#if row.usage}
            <Meter
              usedBp={row.usage.meter.used_bp}
              severity={row.usage.meter.severity}
              label={`${providerName[row.key]} plan usage`}
              size="small"
              stale={row.usage.stale}
              decorative
            />
          {:else}
            <Meter
              usedBp={0}
              severity="unknown"
              label={`${providerName[row.key]} plan usage`}
              size="small"
              stale
              decorative
            />
          {/if}
          <div class="who">
            <span class="name"
              >{providerName[row.key]}{#if row.key === answering}<span
                  class="now"
                >
                  · answering</span
                >{/if}</span
            >
            <span class="line">{row.line}</span>
          </div>
          {#if row.usage}
            <span
              class="pct"
              class:alert={row.usage.meter.severity === "warn" ||
                row.usage.meter.severity === "critical"}
              >{percent(row.usage.meter.used_bp)}</span
            >
          {/if}
        </li>
      {/each}
    </ul>
  </section>

  {#if changed.length > 0}
    <section class="block">
      <h2 class="label">Changed in this session</h2>
      <ul class="files">
        {#each changed as file (file.path)}
          <li>
            <span class="file">{file.path}</span>
            <span class="delta"
              >+{file.added} <span class="removed">−{file.removed}</span></span
            >
          </li>
        {/each}
      </ul>
      {#if tokens}<p class="tokens">{tokens} so far</p>{/if}
    </section>
  {/if}
</aside>

<style>
  .rail {
    position: sticky;
    top: 0;
    align-self: stretch;
    min-height: calc(100vh - var(--space-12));
    display: flex;
    flex-direction: column;
    gap: var(--space-8);
    padding: var(--space-6);
    border-inline-start: var(--rule) solid var(--color-rule);
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .label {
    margin: 0;
    color: var(--color-faint);
    font-size: var(--text-micro);
    font-weight: var(--text-sm-weight-strong);
    line-height: var(--text-micro-line);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .folder {
    justify-content: space-between;
    margin-inline: calc(var(--space-2) * -1);
    font-weight: var(--text-md-weight);
  }
  .path {
    font-family: var(--font-mono);
    overflow-wrap: anywhere;
    text-align: start;
  }
  .change {
    color: var(--color-muted);
    font-size: var(--text-micro);
  }
  .providers,
  .files {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .provider {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    margin-inline: calc(var(--space-2) * -1);
    padding: var(--space-2);
    border-radius: var(--radius-control);
    transition: background-color var(--duration-base) var(--ease);
  }
  .provider.answering {
    background: var(--color-chip);
  }
  .provider.off {
    color: var(--color-faint);
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: var(--text-sm);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-sm-line);
  }
  .now {
    font-size: var(--text-micro);
    font-weight: var(--text-sm-weight-strong);
  }
  .line {
    color: var(--color-muted);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
    overflow-wrap: anywhere;
  }
  .pct {
    font-family: var(--font-serif);
    font-size: var(--text-title);
    line-height: var(--text-title-line);
    font-variant-numeric: tabular-nums;
  }
  .pct.alert {
    color: var(--color-alert);
  }
  .files li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    line-height: var(--text-sm-line);
  }
  .file {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .delta {
    flex: none;
    color: var(--color-muted);
  }
  .removed {
    color: var(--color-alert);
  }
  .tokens {
    font-size: var(--text-micro);
    font-variant-numeric: tabular-nums;
  }
</style>
