<!-- Coding-agent chat screen: full multi-provider routing, workspace picker, tool
     approvals, live streaming deltas, and edit diffs in the Waffle design system. -->
<script lang="ts">
  import { providerName } from "../lib/copy";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  import type { ChatDecision } from "../lib/ipc/chat-bindings/ChatDecision";
  import type { ChatDirListing } from "../lib/ipc/chat-bindings/ChatDirListing";
  import type { ChatMode } from "../lib/ipc/chat-bindings/ChatMode";
  import type { ChatPin } from "../lib/ipc/chat-bindings/ChatPin";
  import { tick } from "svelte";
  import Markdown from "../lib/components/chat/Markdown.svelte";
  import { pushEscape } from "../lib/escape";
  import { chat } from "../lib/stores/chat.svelte";

  const modes: readonly { value: ChatMode; label: string }[] = [
    { value: "ask", label: "Ask (every edit & cmd)" },
    { value: "edit", label: "Edit (accept edits)" },
    { value: "plan", label: "Plan (read-only)" },
    { value: "auto", label: "Auto (unattended)" },
  ];

  const pinProviders: readonly ProviderKey[] = [
    "claude",
    "codex",
    "gemini",
    "grok",
    "deepseek",
  ];

  let errorText = $state<string | null>(null);
  let pickerOpen = $state(false);
  let dirInput = $state("");
  let listing = $state<ChatDirListing | null>(null);
  let pickerError = $state<string | null>(null);

  const pinKey = $derived.by(() => {
    const p = chat.state.pin;
    if (p.kind === "auto") return "auto";
    if (p.kind === "provider") return `provider:${p.provider}`;
    return `model:${p.provider}:${p.model}`;
  });

  function parsePinKey(value: string): ChatPin {
    if (value === "auto") return { kind: "auto" };
    if (value.startsWith("provider:")) {
      const name = value.slice("provider:".length);
      const provider = pinProviders.find((k) => k === name);
      if (provider !== undefined) return { kind: "provider", provider };
    }
    if (value.startsWith("model:")) {
      const rest = value.slice("model:".length);
      const colon = rest.indexOf(":");
      if (colon > 0) {
        const provName = rest.slice(0, colon);
        const model = rest.slice(colon + 1);
        const provider = pinProviders.find((k) => k === provName);
        if (provider !== undefined && model.length > 0) {
          return { kind: "model", provider, model };
        }
      }
    }
    return { kind: "auto" };
  }

  function parseModeValue(value: string): ChatMode {
    const hit = modes.find((m) => m.value === value);
    return hit ? hit.value : "edit";
  }

  async function submitPrompt(): Promise<void> {
    const text = chat.draft.trim();
    if (text.length === 0 || chat.state.busy) return;
    errorText = null;
    chat.draft = "";
    followLatest = true;
    const res = await chat.send(text);
    if (res.status === "error") {
      chat.draft = text;
      errorText =
        res.error.code === "busy"
          ? "A turn is already running."
          : "Could not send message.";
    }
  }

  function onComposerKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submitPrompt();
    }
  }

  // Esc closes the folder picker first, before it would leave the Chat view.
  $effect(() => {
    if (!pickerOpen) return;
    return pushEscape("disclosure", () => {
      pickerOpen = false;
      return true;
    });
  });

  // Follow new output while the reader is at the bottom (or has just sent); someone who
  // scrolled up to read is left where they are.
  let followLatest = true;
  let lastActivity = "";
  const activity = $derived(
    [
      chat.state.messages.length,
      chat.state.streaming_text.length,
      chat.state.streaming_tools.length,
      chat.state.notes.length,
      chat.state.pending_permission?.request_id ?? "",
      chat.state.busy,
    ].join("|"),
  );
  $effect.pre(() => {
    if (activity === lastActivity) return;
    lastActivity = activity;
    const page = document.scrollingElement ?? document.documentElement;
    const atBottom =
      page.scrollHeight - page.scrollTop - page.clientHeight < 160;
    if (!atBottom && !followLatest) return;
    followLatest = false;
    void tick().then(() => {
      page.scrollTop = page.scrollHeight;
    });
  });

  async function openFolderPicker(): Promise<void> {
    pickerError = null;
    dirInput = chat.state.workspace;
    pickerOpen = true;
    const res = await chat.listDirs(null);
    if (res.status === "ok") {
      listing = res.value;
      dirInput = res.value.current;
    } else {
      pickerError = "Could not list workspace directory.";
    }
  }

  async function browseDir(path: string | null): Promise<void> {
    pickerError = null;
    const res = await chat.listDirs(path);
    if (res.status === "ok") {
      listing = res.value;
      dirInput = res.value.current;
    } else {
      pickerError = "Folder not found or unreadable.";
    }
  }

  async function applyWorkspace(path: string): Promise<void> {
    pickerError = null;
    const res = await chat.setWorkspace(path);
    if (res.status === "ok") {
      pickerOpen = false;
    } else {
      pickerError =
        res.error.code === "busy"
          ? "Stop the active turn before switching workspaces."
          : "Directory does not exist.";
    }
  }

  function folderName(path: string): string {
    const parts = path.split("/").filter((s) => s.length > 0);
    return parts[parts.length - 1] ?? path;
  }

  async function answerPerm(decision: ChatDecision): Promise<void> {
    const pending = chat.state.pending_permission;
    if (!pending) return;
    await chat.answerPermission({
      request_id: pending.request_id,
      decision,
    });
  }
</script>

<div class="chat-shell">
  <div class="top-row">
    <h1 id="view-title" tabindex="-1">Coding chat</h1>
    <div class="top-actions">
      <button
        type="button"
        class="small"
        disabled={chat.state.busy}
        onclick={openFolderPicker}
      >
        Folder: {folderName(chat.state.workspace || ".")}
      </button>
      <button
        type="button"
        class="small"
        disabled={chat.state.busy || chat.state.messages.length === 0}
        onclick={() => {
          void chat.newSession();
        }}>New session</button
      >
    </div>
  </div>

  <div class="controls-bar" aria-label="Session controls">
    <label class="field">
      <span>Mode</span>
      <select
        aria-label="Permission mode"
        value={chat.state.mode}
        onchange={(event) => {
          if (event.currentTarget instanceof HTMLSelectElement) {
            void chat.setMode(parseModeValue(event.currentTarget.value));
          }
        }}
      >
        {#each modes as mode (mode.value)}
          <option value={mode.value}>{mode.label}</option>
        {/each}
      </select>
    </label>

    <label class="field">
      <span>Model</span>
      <select
        aria-label="Model routing"
        value={pinKey}
        onchange={(event) => {
          if (event.currentTarget instanceof HTMLSelectElement) {
            void chat.setPin(parsePinKey(event.currentTarget.value));
          }
        }}
      >
        <option value="auto">Auto (Jev routed)</option>
        <optgroup label="By provider">
          {#each pinProviders as prov (prov)}
            <option value={`provider:${prov}`}
              >{providerName[prov]} (auto)</option
            >
          {/each}
        </optgroup>
        <optgroup label="Specific model">
          {#each chat.state.models as model (model.id)}
            <option value={`model:${model.provider}:${model.id}`}>
              {providerName[model.provider]} · {model.id}
            </option>
          {/each}
        </optgroup>
      </select>
    </label>

    <label class="orch-toggle">
      <input
        type="checkbox"
        checked={chat.state.orchestrate}
        onchange={(event) => {
          if (event.currentTarget instanceof HTMLInputElement) {
            void chat.setOrchestrate(event.currentTarget.checked);
          }
        }}
      />
      <span>Orchestrate</span>
    </label>
  </div>

  <div class="providers-strip" aria-label="Provider availability">
    {#each chat.state.providers as prov (prov.provider)}
      <span class="prov-pill" data-ready={prov.ready}>
        <span class="dot" aria-hidden="true"></span>
        {providerName[prov.provider]}
      </span>
    {/each}
    {#if chat.loaded && !chat.state.jev_key_present}
      <span class="key-note">No JEV_API_KEY — fallback routing</span>
    {/if}
  </div>

  {#if pickerOpen}
    <section class="picker-panel" aria-label="Choose workspace folder">
      <div class="picker-head">
        <h2>Workspace folder</h2>
        <button
          type="button"
          class="small text"
          onclick={() => {
            pickerOpen = false;
          }}>Close</button
        >
      </div>
      <div class="picker-input-row">
        <input type="text" aria-label="Workspace path" bind:value={dirInput} />
        <button
          type="button"
          class="small"
          onclick={() => {
            void browseDir(dirInput);
          }}>Browse</button
        >
        <button
          type="button"
          class="small primary"
          onclick={() => {
            void applyWorkspace(dirInput);
          }}>Select</button
        >
      </div>
      {#if pickerError}
        <p class="error-line">{pickerError}</p>
      {/if}
      {#if listing}
        <div class="dir-list">
          {#if listing.parent}
            <button
              type="button"
              class="dir-item text"
              onclick={() => {
                if (listing?.parent) void browseDir(listing.parent);
              }}>.. (Parent directory)</button
            >
          {/if}
          {#each listing.dirs as dir (dir)}
            <button
              type="button"
              class="dir-item text"
              onclick={() => {
                void browseDir(dir);
              }}>{folderName(dir)}</button
            >
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  <div class="transcript" role="log" aria-label="Conversation">
    {#if chat.state.messages.length === 0 && !chat.state.busy}
      <div class="empty-chat">
        <p>
          Working in <code>{chat.state.workspace || "."}</code>. Ask a coding
          question, request edits, or run checks across providers.
        </p>
      </div>
    {/if}

    {#each chat.state.messages as msg (msg.id)}
      <article class="msg" data-role={msg.role}>
        <header class="msg-head">
          <span class="role-label">
            {msg.role === "user"
              ? "You"
              : msg.provider
                ? `${providerName[msg.provider]}${msg.model ? ` · ${msg.model}` : ""}`
                : "Assistant"}
          </span>
          {#if msg.status !== "answered"}
            <span class="status-tag">{msg.status}</span>
          {/if}
        </header>

        {#if msg.route_reason}
          <p class="route-line">{msg.route_reason}</p>
        {/if}

        {#if msg.thinking}
          <details class="thinking-box">
            <summary>Reasoning</summary>
            <pre class="thinking-body">{msg.thinking}</pre>
          </details>
        {/if}

        {#if msg.tool_calls.length > 0}
          <ul class="tool-list">
            {#each msg.tool_calls as tool, idx (idx)}
              <li class="tool-item">
                <div class="tool-summary">
                  <span class="tool-badge" data-ok={tool.ok}>
                    {tool.ok === true ? "✓" : tool.ok === false ? "✗" : "·"}
                  </span>
                  <span>{tool.summary}</span>
                </div>
                {#if tool.output_preview}
                  <pre class="tool-preview">{tool.output_preview}</pre>
                {/if}
                {#if tool.edit}
                  <div class="diff-block">
                    <div class="diff-path">{tool.edit.path}</div>
                    {#each tool.edit.removed as line, lidx (lidx)}
                      <div class="diff-rem">- {line}</div>
                    {/each}
                    {#each tool.edit.added as line, lidx (lidx)}
                      <div class="diff-add">+ {line}</div>
                    {/each}
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}

        {#if msg.role === "user"}
          <div class="msg-body">{msg.content}</div>
        {:else}
          <div class="msg-body md"><Markdown text={msg.content} /></div>
        {/if}

        {#if msg.changed_files.length > 0}
          <div class="changed-files">
            Changed: {msg.changed_files.join(", ")}
          </div>
        {/if}

        {#if msg.usage}
          <div class="usage-line">
            in {msg.usage.input_tokens} · out {msg.usage.output_tokens} · cached {msg
              .usage.cache_read_tokens}
          </div>
        {/if}
      </article>
    {/each}

    {#if chat.state.busy}
      <article class="msg streaming" data-role="assistant">
        <header class="msg-head">
          <span class="role-label">
            {#if chat.state.active_route}
              {providerName[chat.state.active_route.provider]} · {chat.state
                .active_route.model}
            {:else}
              Routing turn...
            {/if}
          </span>
        </header>

        {#if chat.state.active_route}
          <p class="route-line">{chat.state.active_route.reason}</p>
        {/if}

        {#each chat.state.notes as note, idx (idx)}
          <p class="note-line">{note}</p>
        {/each}

        {#if chat.state.streaming_tools.length > 0}
          <ul class="tool-list">
            {#each chat.state.streaming_tools as tool, idx (idx)}
              <li class="tool-item">
                <div class="tool-summary">
                  <span class="tool-badge" data-ok={tool.ok}>
                    {tool.ok === true ? "✓" : tool.ok === false ? "✗" : "…"}
                  </span>
                  <span>{tool.summary}</span>
                </div>
                {#if tool.output_preview}
                  <pre class="tool-preview">{tool.output_preview}</pre>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}

        {#if chat.state.streaming_text.length > 0}
          <div class="msg-body md">
            <Markdown text={chat.state.streaming_text} />
          </div>
        {/if}
      </article>
    {/if}
  </div>

  {#if chat.state.pending_permission}
    {@const perm = chat.state.pending_permission}
    <section class="perm-card" aria-label="Tool approval required">
      <div class="perm-title">Approve {perm.tool}?</div>
      <pre class="perm-summary">{perm.summary}</pre>
      {#if perm.blocked_path}
        <p class="perm-blocked">Outside workspace: {perm.blocked_path}</p>
      {/if}
      <div class="perm-actions">
        <button
          type="button"
          class="primary small"
          onclick={() => {
            void answerPerm("allow");
          }}>Allow once</button
        >
        {#if perm.can_always}
          <button
            type="button"
            class="small"
            onclick={() => {
              void answerPerm("allow_always");
            }}
          >
            Always{perm.always_label ? ` (${perm.always_label})` : ""}
          </button>
        {/if}
        <button
          type="button"
          class="small"
          onclick={() => {
            void answerPerm("deny");
          }}>Deny</button
        >
      </div>
    </section>
  {/if}

  {#if errorText}
    <p class="error-line">{errorText}</p>
  {/if}

  <form
    class="composer"
    onsubmit={(event) => {
      event.preventDefault();
      void submitPrompt();
    }}
  >
    <textarea
      aria-label="Prompt"
      rows="3"
      placeholder="Ask, edit code, or run checks (Enter to send, Shift+Enter for newline)..."
      bind:value={chat.draft}
      onkeydown={onComposerKeydown}
      disabled={chat.state.busy}></textarea>
    <div class="composer-actions">
      {#if chat.state.busy}
        <button
          type="button"
          onclick={() => {
            void chat.cancel();
          }}>Stop</button
        >
      {:else}
        <button
          type="submit"
          class="primary"
          disabled={chat.draft.trim().length === 0}>Send</button
        >
      {/if}
    </div>
  </form>
</div>

<style>
  .chat-shell {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 56rem;
    margin-inline: auto;
    padding: var(--space-4);
  }
  .top-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
    font-weight: var(--title-xl-weight);
  }
  h2 {
    font-family: var(--font-sans);
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
  }
  .top-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .controls-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding-bottom: var(--space-3);
    border-bottom: var(--rule) solid var(--color-rule);
  }
  .field {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--color-muted);
  }
  .orch-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .providers-strip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-micro);
    color: var(--color-muted);
  }
  .prov-pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-pill);
    background: var(--color-chip);
  }
  .dot {
    width: var(--space-2);
    height: var(--space-2);
    border-radius: var(--radius-full);
    background: var(--color-faint);
  }
  .prov-pill[data-ready="true"] .dot {
    background: var(--color-ink);
  }
  .key-note {
    color: var(--color-stale);
  }
  .picker-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: var(--rule) solid var(--color-border-strong);
    border-radius: var(--radius-control);
    background: var(--color-chip);
  }
  .picker-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .picker-input-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .picker-input-row input {
    flex: 1 1 14rem;
    min-height: var(--control-sm-h);
    padding: var(--space-1) var(--space-2);
    border: var(--rule) solid var(--color-border-strong);
    border-radius: var(--radius-control);
    background: var(--color-bg);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .dir-list {
    display: flex;
    flex-direction: column;
    max-height: 12rem;
    overflow-y: auto;
    border-top: var(--rule) solid var(--color-rule);
    padding-top: var(--space-1);
  }
  .dir-item {
    justify-content: flex-start;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .transcript {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-height: 16rem;
  }
  .empty-chat {
    padding: var(--space-6) var(--space-4);
    border: var(--rule) dashed var(--color-rule);
    border-radius: var(--radius-control);
    text-align: center;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: var(--rule) solid var(--color-rule);
    border-radius: var(--radius-control);
    background: var(--color-bg);
  }
  .msg[data-role="user"] {
    background: var(--color-chip);
  }
  .msg-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-sm);
    font-weight: var(--text-sm-weight-strong);
  }
  .status-tag {
    color: var(--color-alert);
    font-size: var(--text-micro);
  }
  .route-line,
  .note-line,
  .usage-line,
  .changed-files {
    font-size: var(--text-micro);
    color: var(--color-muted);
    font-family: var(--font-mono);
  }
  .msg-body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--text-md);
    line-height: var(--text-md-line);
  }
  .msg-body.md {
    white-space: normal;
  }
  .thinking-box summary {
    cursor: pointer;
    font-size: var(--text-micro);
    color: var(--color-muted);
  }
  .thinking-body,
  .tool-preview,
  .perm-summary {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-small);
    background: var(--color-chip);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .tool-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .tool-item {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
  }
  .tool-summary {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .tool-badge[data-ok="false"] {
    color: var(--color-alert);
  }
  .diff-block {
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-left: var(--focus-ring) solid var(--color-border-strong);
    background: var(--color-chip);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
  }
  .diff-path {
    color: var(--color-muted);
    margin-bottom: var(--space-1);
  }
  .diff-rem {
    color: var(--color-alert);
  }
  .diff-add {
    color: var(--color-ink);
  }
  .perm-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: var(--focus-ring) solid var(--color-alert);
    border-radius: var(--radius-control);
    background: var(--color-chip);
  }
  .perm-title {
    font-weight: var(--text-md-weight-strong);
    color: var(--color-alert);
  }
  .perm-blocked,
  .error-line {
    font-size: var(--text-sm);
    color: var(--color-alert);
  }
  .perm-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .composer {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: var(--rule) solid var(--color-rule);
  }
  textarea {
    width: 100%;
    padding: var(--space-2);
    border: var(--rule) solid var(--color-border-strong);
    border-radius: var(--radius-control);
    background: var(--color-bg);
    color: var(--color-text);
    font-family: var(--font-sans);
    font-size: var(--text-md);
    line-height: var(--text-md-line);
    resize: vertical;
  }
  .composer-actions {
    display: flex;
    justify-content: flex-end;
  }

  @media (min-width: 40rem) {
    .chat-shell {
      padding: var(--space-6);
    }
  }
</style>
