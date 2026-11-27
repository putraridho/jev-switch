<!-- Coding-agent chat (C1): sessions | conversation | this session. Turns put the speaker in
     a gutter, tool calls sit on one timeline, and every reply says what it drew from the
     provider's plan window, so routing and plan usage are visible where the work happens. -->
<script lang="ts">
  import { tick } from "svelte";
  import ApprovalPanel from "../lib/components/chat/ApprovalPanel.svelte";
  import ChatTurn from "../lib/components/chat/ChatTurn.svelte";
  import Markdown from "../lib/components/chat/Markdown.svelte";
  import SessionRail from "../lib/components/chat/SessionRail.svelte";
  import SpendWaffle from "../lib/components/chat/SpendWaffle.svelte";
  import ToolSteps from "../lib/components/chat/ToolSteps.svelte";
  import Switch from "../lib/components/primitives/Switch.svelte";
  import { planUsage, turnSpend, usageByProvider } from "../lib/chat/quota";
  import { routeSentence } from "../lib/chat/route";
  import { changedFiles, sessionTitle, tokensLine } from "../lib/chat/session";
  import { providerName } from "../lib/copy";
  import { pushEscape } from "../lib/escape";
  import { percent, windowName } from "../lib/format";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  import type { ChatDecision } from "../lib/ipc/chat-bindings/ChatDecision";
  import type { ChatDirListing } from "../lib/ipc/chat-bindings/ChatDirListing";
  import type { ChatMessage } from "../lib/ipc/chat-bindings/ChatMessage";
  import type { ChatMode } from "../lib/ipc/chat-bindings/ChatMode";
  import type { ChatPin } from "../lib/ipc/chat-bindings/ChatPin";
  import { chat } from "../lib/stores/chat.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { usage } from "../lib/stores/usage.svelte";

  const modes: readonly { value: ChatMode; name: string; help: string }[] = [
    { value: "ask", name: "Ask", help: "Asks before edits and commands" },
    { value: "edit", name: "Edit", help: "Edits apply, commands ask" },
    { value: "plan", name: "Plan", help: "Read only" },
    { value: "auto", name: "Auto", help: "Runs unattended" },
  ];

  const pinProviders: readonly ProviderKey[] = [
    "claude",
    "codex",
    "gemini",
    "grok",
    "deepseek",
  ];

  const suggestions = [
    "Explain how the router picks a provider",
    "Add a test for the 80% warning mark",
    "Run the checks and fix what fails",
  ];

  let errorText = $state<string | null>(null);
  let pickerOpen = $state(false);
  let dirInput = $state("");
  let listing = $state<ChatDirListing | null>(null);
  let pickerError = $state<string | null>(null);
  let promptBox = $state<HTMLTextAreaElement | null>(null);

  const messages = $derived(chat.state.messages);
  const workspace = $derived(
    chat.state.workspace === "" ? "." : chat.state.workspace,
  );
  const started = $derived(messages.length > 0 || chat.state.busy);
  const title = $derived(sessionTitle(messages));
  const mode = $derived(
    modes.find((m) => m.value === chat.state.mode) ?? modes[1],
  );
  const ready = $derived(
    chat.state.providers
      .filter((p) => p.ready)
      .map((p) => providerName[p.provider]),
  );
  const answering = $derived(
    chat.state.busy ? (chat.state.active_route?.provider ?? null) : null,
  );
  const changed = $derived(changedFiles(messages));
  const sessionTokens = $derived(tokensLine(messages.map((m) => m.usage)));
  const usedProviders = $derived(
    [
      ...new Set(
        messages.flatMap((m) => (m.provider === null ? [] : [m.provider])),
      ),
    ].map((p) => providerName[p]),
  );

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

  // What each reply drew from its provider's plan window: usage remembered when the prompt
  // was sent, against usage when the next prompt is sent (or now, for the latest reply).
  let sentUsage: Partial<Record<ProviderKey, number>> | null = null;
  const costs = $state<
    Record<number, { before: number | undefined; after: number | undefined }>
  >({});
  $effect.pre(() => {
    for (const m of messages) {
      if (m.role !== "assistant" || m.provider === null || m.id in costs)
        continue;
      costs[m.id] = { before: sentUsage?.[m.provider], after: undefined };
      sentUsage = null;
    }
  });
  function freezeCosts(): void {
    const now = usageByProvider(usage.snapshot);
    for (const m of messages) {
      const cost = costs[m.id];
      if (cost && cost.after === undefined && m.provider !== null) {
        cost.after = now[m.provider];
      }
    }
  }
  function spendFor(m: ChatMessage): {
    beforeBp: number;
    afterBp: number;
    window: string;
  } | null {
    if (m.provider === null) return null;
    const current = planUsage(usage.snapshot, m.provider);
    const cost = costs[m.id];
    const before = cost?.before;
    const after = cost?.after ?? current?.meter.used_bp;
    if (
      turnSpend(before, after) === null ||
      before === undefined ||
      after === undefined
    )
      return null;
    return {
      beforeBp: before,
      afterBp: after,
      window: current ? windowName(current.meter.window) : "plan",
    };
  }

  async function submitPrompt(): Promise<void> {
    const text = chat.draft.trim();
    if (text.length === 0 || chat.state.busy) return;
    errorText = null;
    chat.draft = "";
    followLatest = true;
    freezeCosts();
    sentUsage = usageByProvider(usage.snapshot);
    const res = await chat.send(text);
    if (res.status === "error") {
      chat.draft = text;
      sentUsage = null;
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

  function suggest(text: string): void {
    chat.draft = text;
    void tick().then(() => promptBox?.focus());
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
      messages.length,
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

  const statusLine: Record<string, string> = {
    failed: "Failed",
    cancelled: "Stopped. The partial reply is kept.",
    no_route: "No provider could take this turn",
  };
</script>

{#snippet picker()}
  {#if pickerOpen}
    <section class="picker" aria-label="Choose workspace folder">
      <div class="picker-row">
        <input type="text" aria-label="Workspace path" bind:value={dirInput} />
        <button
          type="button"
          class="small primary"
          onclick={() => {
            void applyWorkspace(dirInput);
          }}>Use folder</button
        >
      </div>
      {#if pickerError}
        <p class="error-line">{pickerError}</p>
      {/if}
      {#if listing}
        <div class="dirs">
          {#if listing.parent}
            <button
              type="button"
              class="dir text"
              onclick={() => {
                if (listing?.parent) void browseDir(listing.parent);
              }}>Up one folder</button
            >
          {/if}
          {#each listing.dirs as dir (dir)}
            <button
              type="button"
              class="dir text"
              onclick={() => {
                void browseDir(dir);
              }}>{folderName(dir)}/</button
            >
          {/each}
        </div>
      {/if}
      <div class="picker-foot">
        <span>Agents can only touch files inside this folder.</span>
        <button
          type="button"
          class="small text"
          onclick={() => {
            pickerOpen = false;
          }}>Close</button
        >
      </div>
    </section>
  {/if}
{/snippet}

<div class="chat">
  <aside class="sessions" aria-label="Sessions">
    <div class="sessions-head">
      <h2>Sessions</h2>
      <button
        type="button"
        class="small"
        disabled={chat.state.busy || messages.length === 0}
        onclick={() => {
          void chat.newSession();
        }}>New session</button
      >
    </div>
    {#if started}
      <div class="session" aria-current="true">
        <span class="session-title">{title}</span>
        <span class="session-meta"
          >{usedProviders.length > 0 ? usedProviders.join(", ") : "Routing"} ·
          <span class="mono">{folderName(workspace)}</span></span
        >
      </div>
    {:else}
      <p class="sessions-note">Your first message starts a session.</p>
    {/if}
  </aside>

  <section class="conversation">
    {#if started}
      <header class="chat-head">
        <h1 id="view-title" tabindex="-1">{title}</h1>
        <p class="chat-sub">
          {messages.filter((m) => m.role === "assistant").length === 1
            ? "1 reply"
            : `${String(messages.filter((m) => m.role === "assistant").length)} replies`}
        </p>
      </header>
    {/if}

    <div class="transcript" role="log" aria-label="Conversation">
      {#if !started}
        <div class="empty">
          <h1 id="view-title" tabindex="-1">What should we work on?</h1>
          <p class="empty-lede">
            Each turn goes to whichever provider suits it and has room left.
            {#if ready.length > 0}
              {ready.join(", ")}
              {ready.length === 1 ? "is" : "are"} ready; their plan usage is on the
              right.
            {/if}
          </p>
          <ul class="suggestions">
            {#each suggestions as text (text)}
              <li>
                <button
                  type="button"
                  class="text"
                  onclick={() => {
                    suggest(text);
                  }}><span aria-hidden="true">→</span> {text}</button
                >
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#each messages as msg (msg.id)}
        {#if msg.role === "user"}
          <ChatTurn role="user" who="You">
            <div class="msg-body">{msg.content}</div>
          </ChatTurn>
        {:else}
          {@const spend = spendFor(msg)}
          <ChatTurn
            role="assistant"
            who={msg.provider ? providerName[msg.provider] : "Assistant"}
            model={msg.model}
          >
            {#if msg.route_reason}
              <p class="route">
                <span aria-hidden="true">↳</span>
                {routeSentence(msg.route_reason)}
              </p>
            {/if}
            {#if msg.status !== "answered"}
              <p class="status" data-status={msg.status}>
                {statusLine[msg.status]}
              </p>
            {/if}
            {#if msg.thinking}
              <details class="reasoning">
                <summary>Reasoning</summary>
                <pre>{msg.thinking}</pre>
              </details>
            {/if}
            <ToolSteps tools={msg.tool_calls} />
            <div class="msg-body md"><Markdown text={msg.content} /></div>
            {#if spend !== null || msg.usage !== null || msg.changed_files.length > 0}
              <div class="cost">
                {#if spend}<SpendWaffle
                    beforeBp={spend.beforeBp}
                    afterBp={spend.afterBp}
                  />{/if}
                <span>
                  {#if spend && msg.provider}
                    Used <strong
                      >{percent(spend.afterBp - spend.beforeBp)}</strong
                    >
                    of {providerName[msg.provider]}'s {spend.window} window ({percent(
                      spend.beforeBp,
                    )} → {percent(spend.afterBp)})
                  {/if}
                  {#if msg.usage}{spend ? " · " : ""}{tokensLine([
                      msg.usage,
                    ])}{/if}
                  {#if msg.changed_files.length > 0}
                    {spend !== null || msg.usage !== null ? " · " : ""}Changed
                    <span class="mono">{msg.changed_files.join(", ")}</span>
                  {/if}
                </span>
              </div>
            {/if}
          </ChatTurn>
        {/if}
      {/each}

      {#if chat.state.busy}
        <ChatTurn
          role="assistant"
          who={chat.state.active_route
            ? providerName[chat.state.active_route.provider]
            : "…"}
          model={chat.state.active_route?.model ?? null}
          live
        >
          <p class="route">
            {#if chat.state.active_route}
              <span aria-hidden="true">↳</span>
              {routeSentence(chat.state.active_route.reason)}
            {:else}
              <span class="dots" aria-hidden="true"
                ><span></span><span></span><span></span></span
              >
              Choosing who answers{ready.length > 0
                ? ` — ${ready.join(", ")}`
                : ""}
            {/if}
          </p>
          {#each chat.state.notes as note, idx (idx)}
            <p class="note">{note}</p>
          {/each}
          <ToolSteps
            tools={chat.state.streaming_tools}
            live
            waiting={chat.state.pending_permission?.summary ?? null}
          />
          {#if chat.state.streaming_text.length > 0}
            <div class="msg-body md">
              <Markdown text={chat.state.streaming_text} />
            </div>
          {/if}
        </ChatTurn>
      {/if}
    </div>

    <div class="dock">
      {#if chat.state.pending_permission}
        <ApprovalPanel
          prompt={chat.state.pending_permission}
          who={chat.state.active_route
            ? providerName[chat.state.active_route.provider]
            : "The agent"}
          modeLabel={`${mode?.name ?? ""} mode`}
          onanswer={(decision: ChatDecision) => {
            void answerPerm(decision);
          }}
        />
      {:else}
        <form
          class="composer"
          onsubmit={(event) => {
            event.preventDefault();
            void submitPrompt();
          }}
        >
          <textarea
            aria-label="Prompt"
            rows="2"
            placeholder={chat.state.busy
              ? "Working… Stop to change course."
              : "Ask, edit or run checks…"}
            bind:this={promptBox}
            bind:value={chat.draft}
            onkeydown={onComposerKeydown}
            disabled={chat.state.busy}></textarea>
          <div class="composer-foot">
            <div class="modes" role="group" aria-label="Permission mode">
              {#each modes as m (m.value)}
                <button
                  type="button"
                  aria-pressed={chat.state.mode === m.value}
                  onclick={() => {
                    void chat.setMode(m.value);
                  }}>{m.name}</button
                >
              {/each}
            </div>
            <span class="mode-help">{mode?.help}</span>
            <label class="model">
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
                <option value="auto">Auto</option>
                <optgroup label="By provider">
                  {#each pinProviders as prov (prov)}
                    <option value={`provider:${prov}`}
                      >{providerName[prov]}</option
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
            <Switch
              label="Orchestrate"
              checked={chat.state.orchestrate}
              placement="start"
              onchange={(checked: boolean) => {
                void chat.setOrchestrate(checked);
              }}
            />
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
      {/if}
      {#if errorText}
        <p class="error-line">{errorText}</p>
      {/if}
      {#if chat.loaded && !chat.state.jev_key_present}
        <p class="key-note">
          No JEV_API_KEY: routing falls back to simple rules.
        </p>
      {/if}
    </div>
  </section>

  <SessionRail
    workspace={chat.state.workspace}
    folderLabel={`Folder: ${folderName(workspace)}`}
    folderDisabled={chat.state.busy}
    onchangefolder={() => {
      void openFolderPicker();
    }}
    {picker}
    providers={chat.state.providers}
    snapshot={usage.snapshot}
    now={clock.now}
    {answering}
    {changed}
    tokens={sessionTokens}
  />
</div>

<style>
  .chat {
    display: grid;
    grid-template-columns: var(--chat-sessions-w) minmax(0, 1fr) var(
        --chat-rail-w
      );
    align-items: start;
    min-height: calc(100vh - var(--space-12));
  }

  /* Sessions */
  .sessions {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-height: calc(100vh - var(--space-12));
    padding: var(--space-6) var(--space-4);
    border-inline-end: var(--rule) solid var(--color-rule);
  }
  .sessions-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: 0 var(--space-2) var(--space-3);
  }
  .sessions-head h2 {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-sm-line);
  }
  .session {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    border-radius: var(--radius-control);
    background: var(--color-chip);
  }
  .session::before {
    content: "";
    position: absolute;
    inset-block: var(--space-2);
    inset-inline-start: 0;
    border-inline-start: var(--tab-indicator) solid var(--color-ink);
  }
  .session-title {
    font-size: var(--text-sm);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-sm-line);
    overflow-wrap: anywhere;
  }
  .session-meta,
  .sessions-note {
    color: var(--color-muted);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  .sessions-note {
    padding-inline: var(--space-2);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* Conversation */
  .conversation {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: calc(100vh - var(--space-12));
    padding: var(--space-8) var(--space-12) 0;
  }
  .chat-head {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-block-end: var(--space-6);
  }
  .chat-head h1 {
    margin: 0;
    font-family: var(--font-serif);
    font-size: var(--title-lg);
    font-weight: var(--title-xl-weight);
    line-height: var(--title-lg-line);
  }
  .chat-sub {
    font-size: var(--text-micro);
  }
  .transcript {
    display: flex;
    flex: 1;
    flex-direction: column;
  }

  .empty {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-block-start: var(--space-14);
    animation: rise var(--duration-spring-firm) var(--ease-spring-firm) both;
  }
  .empty h1 {
    margin: 0;
    max-width: 12ch;
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    font-weight: var(--num-xl-weight);
    line-height: var(--num-xl-line);
  }
  .empty-lede {
    max-width: 34rem;
    font-size: var(--text-md);
    line-height: var(--text-lg-line);
  }
  .suggestions {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    margin: var(--space-2) 0 0;
    padding: 0;
    list-style: none;
  }
  .suggestions button {
    margin-inline-start: calc(var(--space-2) * -1);
    font-size: var(--text-md);
    font-weight: var(--text-md-weight);
  }
  .suggestions span {
    color: var(--color-faint);
  }

  .msg-body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .msg-body.md {
    white-space: normal;
  }
  .route,
  .note {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  /* These sit in a turn, whose <p> is reading text: put them back to captions. */
  .transcript .route,
  .transcript .note,
  .transcript .status {
    color: var(--color-muted);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  .transcript .status[data-status="failed"],
  .transcript .status[data-status="no_route"] {
    color: var(--color-alert);
  }
  .transcript .status[data-status="cancelled"] {
    color: var(--color-stale);
  }
  .reasoning summary {
    min-height: auto;
    font-size: var(--text-micro);
  }
  .reasoning pre {
    margin: var(--space-2) 0 0;
    color: var(--color-muted);
    font-family: var(--font-sans);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    white-space: pre-wrap;
  }
  .cost {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    color: var(--color-muted);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  .cost strong {
    color: var(--color-ink);
    font-weight: var(--text-sm-weight-strong);
  }
  .dots {
    display: inline-flex;
    gap: var(--space-1);
  }
  .dots span {
    width: calc(var(--space-1) + var(--rule));
    height: calc(var(--space-1) + var(--rule));
    border-radius: var(--radius-full);
    background: var(--color-muted);
    animation: pulse var(--duration-slow) var(--ease) infinite alternate;
  }
  .dots span:nth-child(2) {
    animation-delay: var(--duration-fast);
  }
  .dots span:nth-child(3) {
    animation-delay: var(--duration-base);
  }

  /* Dock: the composer, or an approval in its place, pinned to the bottom of the window. */
  .dock {
    position: sticky;
    bottom: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-block: var(--space-4) var(--space-6);
    background: var(--color-paper);
  }
  .composer {
    display: flex;
    flex-direction: column;
    border: var(--outline) solid var(--color-faint);
    border-radius: var(--radius-dialog);
    background: var(--color-paper);
    transition: border-color var(--duration-fast) var(--ease);
  }
  .composer:focus-within {
    border-color: var(--color-ink);
  }
  textarea {
    resize: none;
    padding: var(--space-3) var(--space-4) var(--space-1);
    border: 0;
    background: transparent;
    color: var(--color-ink);
    font: inherit;
    font-size: var(--text-lg);
    line-height: var(--text-lg-line);
  }
  textarea:focus-visible {
    outline: none;
  }
  .composer-foot {
    display: flex;
    flex-wrap: nowrap;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
  }
  .modes {
    display: inline-flex;
    padding: var(--segment-inset);
    border-radius: var(--segment-radius);
    background: var(--color-chip);
  }
  .modes button {
    min-height: auto;
    min-width: auto;
    height: var(--space-6);
    padding-inline: var(--space-3);
    border-color: transparent;
    border-radius: var(--radius-pill);
    color: var(--color-muted);
    font-size: var(--text-micro);
  }
  .modes button[aria-pressed="true"] {
    background: var(--color-paper);
    color: var(--color-ink);
  }
  /* The help line gives way first when the composer is narrow. */
  .mode-help {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--color-muted);
    font-size: var(--text-micro);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .model {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-muted);
    font-size: var(--text-micro);
  }
  .model select {
    width: auto;
    max-width: 8.5rem;
    min-height: var(--control-sm-h);
    font-size: var(--text-micro);
  }
  .error-line {
    color: var(--color-alert);
  }
  .key-note {
    color: var(--color-stale);
    font-size: var(--text-micro);
  }

  /* Folder picker, under the folder in the rail */
  .picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: var(--rule) solid var(--color-rule);
    border-radius: var(--radius-dialog);
    background: var(--color-paper);
    animation: drop var(--duration-spring-heavy) var(--ease-spring-heavy) both;
  }
  .picker-row {
    display: flex;
    gap: var(--space-2);
  }
  .picker-row input {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
  }
  .dirs {
    display: flex;
    flex-direction: column;
    border-top: var(--rule) solid var(--color-rule);
    padding-block-start: var(--space-1);
  }
  .dir {
    justify-content: flex-start;
    font-family: var(--font-mono);
    font-weight: var(--text-md-weight);
  }
  .picker-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    color: var(--color-muted);
    font-size: var(--text-micro);
  }

  @keyframes rise {
    from {
      opacity: 0;
      translate: 0 var(--space-2);
    }
    to {
      opacity: 1;
      translate: 0 0;
    }
  }
  @keyframes drop {
    from {
      clip-path: inset(0 0 100% 0 round var(--radius-dialog));
      translate: 0 calc(var(--space-2) * -1);
    }
    to {
      clip-path: inset(0 0 0 0 round var(--radius-dialog));
      translate: 0 0;
    }
  }
  @keyframes pulse {
    to {
      opacity: 0.25;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dots span {
      animation: none;
    }
  }

  /* Narrower windows: the sessions list goes first, then the rail drops below. */
  @media (max-width: 74.99rem) {
    .chat {
      grid-template-columns: minmax(0, 1fr) var(--chat-rail-w);
    }
    .sessions {
      display: none;
    }
  }
  @media (max-width: 59.99rem) {
    .chat {
      grid-template-columns: minmax(0, 1fr);
    }
    .conversation {
      padding-inline: var(--space-6);
    }
  }
</style>
