<!-- A turn's tool calls on one timeline (C1): a mark per step (done, failed, running, waiting
     for approval), the step's summary in mono, its output and any edit hanging off it. -->
<script lang="ts">
  import type { ChatToolCall } from "../../ipc/chat-bindings/ChatToolCall";

  interface Props {
    tools: readonly ChatToolCall[];
    /** The turn is still running: a step with no result yet is running, not unknown. */
    live?: boolean;
    /** A step waiting on the person's approval, shown last. */
    waiting?: string | null;
  }

  const { tools, live = false, waiting = null }: Props = $props();

  function state(tool: ChatToolCall): "done" | "failed" | "running" | "plain" {
    if (tool.ok === true) return "done";
    if (tool.ok === false) return "failed";
    return live ? "running" : "plain";
  }

  const label: Record<string, string> = {
    done: "done",
    failed: "failed",
    running: "running",
    plain: "",
    waiting: "waiting for your approval",
  };
</script>

{#if tools.length > 0 || waiting !== null}
  <ol class="steps" aria-label="Steps">
    {#each tools as tool, idx (idx)}
      {@const s = state(tool)}
      <li class="step" data-state={s}>
        <span class="mark" aria-hidden="true"></span>
        <div class="what">
          <span class="summary">{tool.summary}</span>
          {#if label[s]}<span class="visually-hidden">, {label[s]}</span>{/if}
          {#if tool.output_preview}
            <pre class="output">{tool.output_preview}</pre>
          {/if}
          {#if tool.edit}
            <div
              class="diff"
              role="group"
              aria-label={`Edit to ${tool.edit.path}`}
            >
              <div class="diff-head">
                <span class="path">{tool.edit.path}</span>
                <span class="count"
                  >+{tool.edit.added.length}
                  <span class="removed">−{tool.edit.removed.length}</span></span
                >
              </div>
              {#each tool.edit.removed as line, lidx (lidx)}
                <div class="line rem">
                  <span aria-hidden="true">−</span>
                  {line}
                </div>
              {/each}
              {#each tool.edit.added as line, lidx (lidx)}
                <div class="line add">
                  <span aria-hidden="true">+</span>
                  {line}
                </div>
              {/each}
            </div>
          {/if}
        </div>
      </li>
    {/each}
    {#if waiting !== null}
      <li class="step" data-state="waiting">
        <span class="mark" aria-hidden="true"></span>
        <div class="what">
          <span class="summary">{waiting}</span>
          <span class="note">Waiting for you</span>
        </div>
      </li>
    {/if}
  </ol>
{/if}

<style>
  .steps {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .step {
    position: relative;
    display: grid;
    grid-template-columns: var(--space-6) minmax(0, 1fr);
    padding-block-end: var(--space-3);
    animation: land var(--duration-spring-firm) var(--ease-spring-firm) both;
  }
  .step:last-child {
    padding-block-end: 0;
  }
  /* The timeline: a hairline from each mark down to the next. */
  .step:not(:last-child)::before {
    content: "";
    position: absolute;
    inset-block: calc(var(--space-4) + var(--space-1)) 0;
    inset-inline-start: calc(var(--space-3) - var(--rule));
    border-inline-start: var(--rule) solid var(--color-rule);
  }
  .mark {
    position: relative;
    justify-self: center;
    width: var(--step-mark);
    height: var(--step-mark);
    margin-block-start: var(--space-2);
    border-radius: var(--radius-full);
    background: var(--color-ink);
  }
  [data-state="failed"] .mark {
    background: var(--color-alert);
  }
  [data-state="plain"] .mark {
    background: var(--color-faint);
  }
  [data-state="waiting"] .mark {
    background: transparent;
    box-shadow: inset 0 0 0 var(--switch-border) var(--color-stale);
  }
  [data-state="running"] .mark::after {
    content: "";
    position: absolute;
    inset: 0;
    border: var(--switch-border) solid var(--color-ink);
    border-radius: var(--radius-full);
    animation: ping var(--duration-slow) var(--ease-in) infinite;
    animation-duration: calc(var(--duration-slow) * 3.5);
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .summary {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
    overflow-wrap: anywhere;
  }
  [data-state="failed"] .summary {
    color: var(--color-alert);
  }
  .note {
    color: var(--color-stale);
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  .output {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-control);
    background: var(--color-chip);
    color: var(--color-muted);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    line-height: var(--text-sm-line);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .diff {
    overflow: hidden;
    border-radius: var(--radius-control);
    background: var(--color-chip);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    line-height: var(--text-sm-line);
  }
  .diff-head {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-1) var(--space-3);
    border-bottom: var(--rule) solid var(--color-rule);
    color: var(--color-muted);
  }
  .path {
    color: var(--color-ink);
    overflow-wrap: anywhere;
  }
  .removed,
  .rem {
    color: var(--color-alert);
  }
  .line {
    padding-inline: var(--space-3);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .line:first-of-type {
    padding-block-start: var(--space-1);
  }
  .line:last-child {
    padding-block-end: var(--space-1);
  }
  .rem {
    text-decoration: line-through;
  }
  .add {
    background: var(--color-hover);
    color: var(--color-ink);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @keyframes land {
    from {
      opacity: 0;
      translate: calc(var(--space-2) * -1) 0;
    }
    to {
      opacity: 1;
      translate: 0 0;
    }
  }
  @keyframes ping {
    from {
      scale: 0.6;
      opacity: 0.9;
    }
    to {
      scale: 1.9;
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    [data-state="running"] .mark::after {
      animation: none;
    }
  }
</style>
