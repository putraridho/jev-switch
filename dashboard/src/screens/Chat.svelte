<!-- Coding-agent chat (C1): sessions | conversation | this session. Turns put the speaker in
     a gutter, tool calls sit on one timeline, and every reply says what it drew from the
     provider's plan window, so routing and plan usage are visible where the work happens. -->
<script module lang="ts">
  import { SvelteMap } from "svelte/reactivity";
  // Arrival times outlive the view (leaving the tab and coming back keeps them).
  const seen = new SvelteMap<number, number | null>();
  let primed = false;
</script>

<script lang="ts">
  import { tick, untrack } from "svelte";
  import ApprovalPanel from "../lib/components/chat/ApprovalPanel.svelte";
  import AttachChip from "../lib/components/chat/AttachChip.svelte";
  import ChatTurn from "../lib/components/chat/ChatTurn.svelte";
  import Markdown from "../lib/components/chat/Markdown.svelte";
  import SessionRail from "../lib/components/chat/SessionRail.svelte";
  import SpendWaffle from "../lib/components/chat/SpendWaffle.svelte";
  import ToolSteps from "../lib/components/chat/ToolSteps.svelte";
  import {
    ACCEPT,
    chipMeta,
    dragHasFiles,
    isLongPaste,
    typeTag,
  } from "../lib/chat/attachments";
  import { planUsage, turnSpend, usageByProvider } from "../lib/chat/quota";
  import { modelLabel, routeSentence } from "../lib/chat/route";
  import { changedFiles, sessionTitle, tokensLine } from "../lib/chat/session";
  import { providerName } from "../lib/copy";
  import { pushEscape } from "../lib/escape";
  import { percent, timeOfDay, windowName } from "../lib/format";
  import type { ProviderKey } from "../lib/ipc/bindings/ProviderKey";
  import type { ChatAttachment } from "../lib/ipc/chat-bindings/ChatAttachment";
  import type { ChatDecision } from "../lib/ipc/chat-bindings/ChatDecision";
  import type { ChatDirListing } from "../lib/ipc/chat-bindings/ChatDirListing";
  import type { ChatMessage } from "../lib/ipc/chat-bindings/ChatMessage";
  import type { ChatMode } from "../lib/ipc/chat-bindings/ChatMode";
  import type { ChatPin } from "../lib/ipc/chat-bindings/ChatPin";
  import {
    pendingMeta,
    previews,
    tray,
  } from "../lib/stores/attachments.svelte";
  import { chat } from "../lib/stores/chat.svelte";
  import { clock } from "../lib/stores/clock.svelte";
  import { settings } from "../lib/stores/settings.svelte";
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
  let transcriptEl = $state<HTMLDivElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);
  // Files dragged over the conversation: a count, since entering a child fires dragenter
  // before the parent's dragleave.
  let dragDepth = $state(0);
  const dropping = $derived(dragDepth > 0);
  // How many files are coming, when the drag says (the browser may not until the drop).
  let dragFiles = $state(0);

  const messages = $derived(chat.state.messages);
  const workspace = $derived(
    chat.state.workspace === "" ? "." : chat.state.workspace,
  );
  const started = $derived(messages.length > 0 || chat.state.busy);
  const title = $derived(sessionTitle(messages, chat.state.title));
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

  // When each entry arrived. The chat sends no times, so the screen notes the moment it
  // first sees an entry; entries already there when the app opened stay untimed.
  $effect.pre(() => {
    const now = Date.now();
    const ids = messages.map((m) => m.id);
    untrack(() => {
      for (const id of ids) {
        if (!seen.has(id)) seen.set(id, primed ? now : null);
      }
      primed = true;
    });
  });
  function timeOf(id: number): string | null {
    const at = seen.get(id);
    return at == null ? null : timeOfDay(at, settings.value.hour_cycle);
  }
  const replies = $derived(
    messages.filter((m) => m.role === "assistant").length,
  );
  const startedAt = $derived.by(() => {
    const first = messages.find((m) => m.role === "user");
    return first ? timeOf(first.id) : null;
  });

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

  // Something to send (words or attachments), and nothing in the tray holding it up.
  const canSend = $derived(
    !chat.state.busy &&
      tray.clear &&
      (chat.draft.trim().length > 0 || tray.ready),
  );

  async function submitPrompt(): Promise<void> {
    if (!canSend) return;
    const text = chat.draft.trim();
    const attached: ChatAttachment[] = tray.attachments;
    errorText = null;
    chat.draft = "";
    tray.clearAll();
    followLatest = true;
    freezeCosts();
    sentUsage = usageByProvider(usage.snapshot);
    const res = await chat.send(
      text,
      attached.map((a) => a.path),
    );
    if (res.status === "error") {
      chat.draft = text;
      tray.restore(attached);
      sentUsage = null;
      errorText =
        res.error.code === "busy"
          ? "A turn is already running."
          : attached.length > 0
            ? "Could not send. An attachment may have changed or moved."
            : "Could not send message.";
    }
  }

  function attachFiles(list: FileList | null | undefined): void {
    if (!list || list.length === 0) return;
    void tray.addFiles([...list]);
  }

  // Pasted pictures and files join the tray; long text does too, so the prompt stays short.
  function onComposerPaste(event: ClipboardEvent): void {
    const data = event.clipboardData;
    if (!data) return;
    if (data.files.length > 0) {
      event.preventDefault();
      attachFiles(data.files);
      return;
    }
    const text = data.getData("text/plain");
    if (isLongPaste(text)) {
      event.preventDefault();
      void tray.addText(text);
    }
  }

  function onDragEnter(event: DragEvent): void {
    if (!dragHasFiles([...(event.dataTransfer?.types ?? [])])) return;
    event.preventDefault();
    if (chat.state.busy || chat.state.pending_permission) return;
    if (dragDepth === 0) {
      dragFiles = [...(event.dataTransfer?.items ?? [])].filter(
        (item) => item.kind === "file",
      ).length;
    }
    dragDepth += 1;
  }
  function onDragOver(event: DragEvent): void {
    if (!dragHasFiles([...(event.dataTransfer?.types ?? [])])) return;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = dropping ? "copy" : "none";
    }
  }
  function onDragLeave(): void {
    if (dragDepth > 0) dragDepth -= 1;
  }
  function onDrop(event: DragEvent): void {
    event.preventDefault();
    const was = dropping;
    dragDepth = 0;
    if (was) {
      attachFiles(event.dataTransfer?.files);
      void tick().then(() => promptBox?.focus());
    }
  }

  // ⌘U (Ctrl+U) opens the file picker while the composer is there.
  function onWindowKeydown(event: KeyboardEvent): void {
    if (
      (event.metaKey || event.ctrlKey) &&
      !event.altKey &&
      !event.shiftKey &&
      event.key.toLowerCase() === "u" &&
      fileInput !== null &&
      !chat.state.busy &&
      !chat.state.pending_permission
    ) {
      event.preventDefault();
      fileInput.click();
    }
  }

  function trayMeta(item: (typeof tray.items)[number]): string {
    return item.status === "ready"
      ? chipMeta(item.attachment)
      : pendingMeta(item);
  }
  function trayName(item: (typeof tray.items)[number]): string {
    return item.status === "ready" ? item.attachment.name : item.name;
  }
  function tagFor(a: { name: string; kind: string }): string {
    return a.kind === "text" ? "¶" : typeTag(a.name);
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
    const box = transcriptEl;
    if (box === null) return;
    const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 160;
    if (!atBottom && !followLatest) return;
    followLatest = false;
    void tick().then(() => {
      box.scrollTop = box.scrollHeight;
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

<svelte:window onkeydown={onWindowKeydown} />

<div class="chat">
  <aside class="sessions" aria-label="Sessions">
    <div class="sessions-head">
      <h2>Sessions</h2>
      <button
        type="button"
        class="small"
        aria-label="New session"
        disabled={chat.state.busy || messages.length === 0}
        onclick={() => {
          void chat.newSession();
        }}>New</button
      >
    </div>
    {#if started}
      <h3 class="group">Today</h3>
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

  <div
    class="conversation"
    role="region"
    aria-label="Chat"
    ondragenter={onDragEnter}
    ondragover={onDragOver}
    ondragleave={onDragLeave}
    ondrop={onDrop}
  >
    {#if started}
      <header class="chat-head">
        <h1 id="view-title" tabindex="-1">{title}</h1>
        <p class="chat-sub">
          {startedAt ? `Started ${startedAt} · ` : ""}{replies === 1
            ? "1 turn"
            : `${String(replies)} turns`}
        </p>
      </header>
    {/if}

    <div
      class="transcript"
      role="log"
      aria-label="Conversation"
      bind:this={transcriptEl}
    >
      <div class="flow">
        {#if !started}
          <div class="empty">
            <h1 id="view-title" tabindex="-1">What should we work on?</h1>
            <p class="empty-lede">
              Each turn goes to whichever provider suits it and has room left.
              {#if ready.length > 0}
                {ready.join(", ")}
                {ready.length === 1 ? "is" : "are"} ready; their plan usage is on
                the right.
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
            <ChatTurn role="user" who="You" time={timeOf(msg.id)}>
              {#if msg.content.length > 0}
                <div class="msg-body">{msg.content}</div>
              {/if}
              {#if msg.attachments.length > 0}
                <ul class="sent" aria-label="Attachments">
                  {#each msg.attachments as a (a.path)}
                    <li>
                      <AttachChip
                        name={a.name}
                        meta={chipMeta(a)}
                        tag={tagFor(a)}
                        preview={previews.get(a.path) ?? null}
                        large
                      />
                    </li>
                  {/each}
                </ul>
              {/if}
            </ChatTurn>
          {:else}
            {@const spend = spendFor(msg)}
            <ChatTurn
              role="assistant"
              who={msg.provider ? providerName[msg.provider] : "Assistant"}
              model={msg.model ? modelLabel(msg.provider, msg.model) : null}
              time={timeOf(msg.id)}
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
            model={chat.state.active_route
              ? modelLabel(
                  chat.state.active_route.provider,
                  chat.state.active_route.model,
                )
              : null}
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
          class:filled={chat.draft.length > 0 || tray.items.length > 0}
          onsubmit={(event) => {
            event.preventDefault();
            void submitPrompt();
          }}
        >
          {#if tray.items.length > 0}
            <ul class="tray" aria-label="Attachments">
              {#each tray.items as item (item.id)}
                <li>
                  <AttachChip
                    name={trayName(item)}
                    meta={trayMeta(item)}
                    tag={item.status === "ready"
                      ? tagFor(item.attachment)
                      : typeTag(item.name)}
                    status={item.status}
                    preview={item.status === "ready"
                      ? (previews.get(item.attachment.path) ?? null)
                      : null}
                    onremove={() => {
                      tray.remove(item.id);
                      void tick().then(() => promptBox?.focus());
                    }}
                  />
                </li>
              {/each}
            </ul>
          {/if}
          <textarea
            aria-label="Prompt"
            rows="2"
            placeholder={chat.state.busy
              ? "Working… Stop to change course."
              : "Ask, edit or run checks…"}
            bind:this={promptBox}
            bind:value={chat.draft}
            onkeydown={onComposerKeydown}
            onpaste={onComposerPaste}
            disabled={chat.state.busy}></textarea>
          {#if dropping}
            <div class="drop-target" aria-hidden="true">
              <svg class="drop-edge"
                ><rect x="0" y="0" width="100%" height="100%" rx="8" /></svg
              >
              <span class="drop-count"
                >{dragFiles === 1
                  ? "1 file"
                  : dragFiles > 1
                    ? `${String(dragFiles)} files`
                    : "Files"}</span
              >
              <span class="drop-hint">join this message</span>
            </div>
          {/if}
          <div class="composer-foot">
            <input
              class="file-input"
              type="file"
              multiple
              accept={ACCEPT}
              tabindex="-1"
              aria-hidden="true"
              bind:this={fileInput}
              onchange={(event) => {
                if (event.currentTarget instanceof HTMLInputElement) {
                  attachFiles(event.currentTarget.files);
                  event.currentTarget.value = "";
                }
              }}
            />
            <button
              type="button"
              class="attach"
              aria-label="Attach files"
              title="Attach files (⌘U). You can also drop or paste them."
              disabled={chat.state.busy}
              onclick={() => fileInput?.click()}
            >
              <svg viewBox="0 0 16 16" aria-hidden="true"
                ><path d="M8 3v10M3 8h10" /></svg
              >
            </button>
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
            <span class="mode-help" class:note={tray.hasImage}
              >{tray.hasImage
                ? "Sent to a model that sees images"
                : mode?.help}</span
            >
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
            <label class="orchestrate">
              <input
                type="checkbox"
                role="switch"
                checked={chat.state.orchestrate}
                onchange={(event) => {
                  if (event.currentTarget instanceof HTMLInputElement) {
                    void chat.setOrchestrate(event.currentTarget.checked);
                  }
                }}
              />
              <span class="track" aria-hidden="true"></span>
              Orchestrate
            </label>
            {#if chat.state.busy}
              <button
                type="button"
                onclick={() => {
                  void chat.cancel();
                }}>Stop</button
              >
            {:else}
              <button type="submit" class="primary" disabled={!canSend}
                >Send{#if canSend}<span class="key" aria-hidden="true">↵</span
                  >{/if}</button
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
    {#if dropping}
      <div class="drop-veil" aria-hidden="true">
        <p class="drop-title">Drop to attach</p>
        <p class="drop-sub">
          Images, text and code, PDFs · up to 20 MB each · 10 at a time
        </p>
      </div>
    {/if}
  </div>

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
    warnPct={Math.round(settings.value.warn_bp / 100)}
  />
</div>

<style>
  .chat {
    display: grid;
    grid-template-columns: var(--chat-sessions-w) minmax(0, 1fr) var(
        --chat-rail-w
      );
    /* The view fills the window under the header and never scrolls as a page: only the
       conversation scrolls, each rail scrolls on its own, the composer stays put. */
    height: calc(100dvh - var(--space-12) - var(--rule));
    overflow: hidden;
  }

  /* Sessions */
  .sessions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-height: 0;
    overflow-y: auto;
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
  .group {
    margin: 0;
    padding: var(--space-3) var(--space-2) var(--space-1);
    color: var(--color-faint);
    font-size: var(--text-micro);
    font-weight: var(--text-sm-weight-strong);
    line-height: var(--text-micro-line);
    letter-spacing: 0.06em;
    text-transform: uppercase;
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
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
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
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  /* Short conversations sit just above the composer; long ones scroll. An auto margin
     rather than justify-content: flex-end, so the top stays reachable. */
  .flow {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }
  .flow > :global(:first-child) {
    margin-block-start: auto;
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
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-block: var(--space-4) var(--space-6);
    background: var(--color-paper);
  }
  .composer {
    position: relative;
    display: flex;
    flex-direction: column;
    border: var(--outline) solid var(--color-faint);
    border-radius: var(--radius-dialog);
    background: var(--color-paper);
    transition: border-color var(--duration-fast) var(--ease);
  }
  .composer:focus-within,
  .composer.filled {
    border-color: var(--color-ink);
  }

  /* Attachments: the tray above the prompt, chips that wrap. */
  .tray,
  .sent {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .tray {
    padding: var(--space-4) var(--space-4) 0;
  }
  .tray + textarea {
    padding-block-start: var(--space-3);
  }
  .attach {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--control-sm-h);
    min-width: 0;
    height: var(--control-sm-h);
    min-height: 0;
    padding: 0;
    border-color: var(--color-rule);
    border-radius: var(--radius-full);
  }
  .attach svg {
    width: var(--space-4);
    height: var(--space-4);
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: 1.5;
  }
  .file-input {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    opacity: 0;
    pointer-events: none;
  }
  .mode-help.note {
    color: var(--color-stale);
  }

  /* Dropping: the composer's edge becomes a run of dots (the waffle's grain) that follows
     its corners and marches slowly, and the conversation behind fades back. */
  .drop-target {
    position: absolute;
    inset: calc(var(--outline) * -1);
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    border-radius: var(--radius-dialog);
    background: var(--color-paper);
    pointer-events: none;
  }
  .drop-edge {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .drop-edge rect {
    fill: none;
    stroke: var(--color-ink);
    stroke-dasharray: 0 9;
    stroke-linecap: round;
    stroke-width: 2.6;
    animation: march calc(var(--duration-slow) * 5) linear infinite;
  }
  .drop-count {
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
  }
  .drop-hint {
    color: var(--color-muted);
    font-size: var(--text-micro);
  }
  .drop-veil {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding-block-end: var(--space-18);
    pointer-events: none;
    animation: fade-in var(--duration-fast) var(--ease) both;
  }
  .drop-veil::before {
    content: "";
    position: absolute;
    inset: 0;
    background: var(--color-paper);
    opacity: 0.86;
  }
  .drop-title,
  .drop-sub {
    position: relative;
    margin: 0;
  }
  .drop-title {
    color: var(--color-ink);
    font-family: var(--font-serif);
    font-size: var(--title-lg);
    line-height: var(--title-lg-line);
  }
  .drop-sub {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
  /* The composer stays above the veil, where the drop lands. */
  .dock {
    position: relative;
    z-index: 1;
  }
  @keyframes march {
    to {
      stroke-dashoffset: -18;
    }
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
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
  /* The model picker reads as words ("Model Auto ▾"), not a boxed field. */
  .model select {
    /* Sized to the chosen model's name where supported, so the ▾ sits right after it. */
    field-sizing: content;
    width: auto;
    max-width: 9rem;
    min-height: var(--control-sm-h);
    padding-inline: var(--space-1);
    border-color: transparent;
    background-color: transparent;
    color: var(--color-ink);
    font-size: var(--text-micro);
    font-weight: var(--text-sm-weight-strong);
  }
  .model select:hover:not(:disabled) {
    border-color: transparent;
    background-color: var(--color-hover);
  }
  /* Orchestrate: a compact switch that fits the composer's line. */
  .orchestrate {
    position: relative;
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-ink);
    font-size: var(--text-micro);
    font-weight: var(--text-sm-weight-strong);
    cursor: pointer;
  }
  .orchestrate input {
    position: absolute;
    inset: 0;
    margin: 0;
    opacity: 0;
    cursor: pointer;
  }
  .track {
    position: relative;
    width: var(--space-6);
    height: var(--space-4);
    border-radius: var(--radius-pill);
    box-shadow: inset 0 0 0 var(--switch-border) var(--color-faint);
    transition: background-color var(--duration-fast) var(--ease);
  }
  .track::after {
    content: "";
    position: absolute;
    inset-block-start: var(--space-1);
    inset-inline-start: var(--space-1);
    width: var(--space-2);
    height: var(--space-2);
    border-radius: var(--radius-full);
    background: var(--color-faint);
    transition:
      translate var(--duration-spring-firm) var(--ease-spring-firm),
      background-color var(--duration-fast) var(--ease);
  }
  .orchestrate input:checked + .track {
    background: var(--color-ink);
    box-shadow: none;
  }
  .orchestrate input:checked + .track::after {
    background: var(--color-paper);
    translate: var(--space-2) 0;
  }
  .orchestrate input:focus-visible + .track {
    outline: var(--focus-ring) solid var(--color-accent);
    outline-offset: var(--space-1);
  }
  .key {
    margin-inline-start: var(--space-2);
    opacity: 0.6;
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
    .dots span,
    .drop-edge rect,
    .drop-veil {
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
    /* One column: back to page scrolling, with the composer pinned to the bottom. */
    .chat {
      grid-template-columns: minmax(0, 1fr);
      height: auto;
      overflow: visible;
    }
    .transcript {
      overflow: visible;
    }
    .dock {
      position: sticky;
      bottom: 0;
    }
    .conversation {
      padding-inline: var(--space-6);
    }
  }
</style>
