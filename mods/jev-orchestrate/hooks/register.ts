// jev-orchestrate: orchestrate mode for jev-router's "Jev Code" model.
//
// On each prompt, ask jev-gateway (POST /jev/orchestrate) whether the prompt is a
// large task that splits into separate pieces. If it is, attach a brief the user
// doesn't see: plan, delegate each step to a Sonnet subagent, review. The gateway
// sees the brief's marker and runs the main thread on the planner model.
//
// On by default. `/orchestrate` toggles it for this session, `/orchestrate on|off`
// sets it. New sessions start on. Any failure falls through to a normal prompt.

import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import { isHarnessPrompt } from './harness'

const enabled = atom({ plugin: 'jev-orchestrate', key: 'enabled' } as const, true)
const active = atom({ plugin: 'jev-orchestrate', key: 'active' } as const, false)
/// Workers (subagents) the planner started during the orchestrated turn: ids still
/// running, and how many have finished. Drives the status line.
const workers = atom({ plugin: 'jev-orchestrate', key: 'workers' } as const, { running: [] as string[], done: 0 })
const NO_WORKERS = { running: [] as string[], done: 0 }

export function statusText(active: boolean, running: number, done: number): string | undefined {
  if (!active && running === 0) return undefined
  if (running === 0 && done === 0) return 'orchestrating · planning'
  const r = `${running} worker${running === 1 ? '' : 's'} running`
  return running === 0 ? `orchestrating · ${done} done, reviewing` : `orchestrating · ${r}, ${done} done`
}

const DEFAULT_GATEWAY = 'http://127.0.0.1:18780'
const CHECK_TIMEOUT_MS = 5000

export const BRIEF = `<jev-orchestrate>
Orchestrate mode (jev-router): you are running on the expensive planner model, so plan and review, and leave all file edits to cheaper subagents. Don't use Write, Edit or NotebookEdit yourself in this turn, however small the change.
1. Plan: read only what you need to split the work into self-contained steps, each one checkable on its own. If it is really one small step, that is a plan of one step.
2. Delegate: one Agent call per step, so never bundle unrelated steps into one subagent. Give each model: "sonnet" and a complete brief (the files involved, what to change, how to verify it, what to report back). Use model: "opus" only for a step that touches security (auth, permissions, secrets, crypto, untrusted input). Run the steps in parallel: put every step that doesn't depend on another and edits different files into ONE message as several Agent calls, never one call per message. Only a step that needs another's result, or edits the same file, waits for it.
3. Review: when the steps report back, check the result against the request (read the diffs, run the tests or the build) and delegate any fixes the same way.
</jev-orchestrate>`

type Mode = 'toggle' | 'on' | 'off' | 'usage'

export function parseArgs(args: string): Mode {
  const a = args.trim().toLowerCase()
  if (a === '') return 'toggle'
  if (a === 'on' || a === 'off') return a
  return 'usage'
}

async function isJevCode($: EngineInterface): Promise<boolean> {
  return (await $.session.model()).startsWith('jev-code')
}

async function showStatus($: EngineInterface): Promise<void> {
  if (!(await isJevCode($))) {
    $.ui.status(undefined)
    return
  }
  const w = await read($, workers)
  const text = statusText(await read($, active), w.running.length, w.done)
  if (text) {
    $.ui.status(text)
    return
  }
  $.ui.status((await read($, enabled)) ? 'orchestrate: on' : 'orchestrate: off')
}

async function gatewayUrl($: EngineInterface): Promise<string> {
  const base = (await $.env.get('ANTHROPIC_BASE_URL')) ?? DEFAULT_GATEWAY
  return base.replace(/\/+$/, '')
}

/// Whether the gateway says to orchestrate this prompt. False on any error or timeout.
async function shouldOrchestrate($: EngineInterface, prompt: string): Promise<boolean> {
  const check = (async () => {
    const res = await $.http.fetch(`${await gatewayUrl($)}/jev/orchestrate`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ prompt, session: await $.session.id() }),
    })
    if (!res.ok) return false
    const body = JSON.parse(res.text) as { trigger?: unknown }
    return body.trigger === true
  })()
  const timeout = $.clock.sleep(CHECK_TIMEOUT_MS).then(() => false)
  try {
    return await Promise.race([check, timeout])
  } catch {
    return false
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'orchestrate',
      description: 'Orchestrate mode for Jev Code: toggle, or set on/off, for this session',
      argumentHint: '[on|off]',
      immediate: true,
    })
    const started = await next(e)
    await showStatus($)
    return started
  })

  on('command.run', { command: 'orchestrate' }, async ($, e) => {
    const mode = parseArgs(e.args)
    if (mode === 'usage') {
      const now = (await read($, enabled)) ? 'on' : 'off'
      return { text: `Usage: /orchestrate (toggle), /orchestrate on, /orchestrate off. Currently ${now}.` }
    }
    const next = await update($, enabled, was => (mode === 'toggle' ? !was : mode === 'on'))
    await showStatus($)
    const note = (await isJevCode($)) ? '' : ' (it only applies while the model is Jev Code)'
    return { text: `Orchestrate mode ${next ? 'on' : 'off'} for this session${note}.` }
  })

  on('prompt.submit', async ($, e, next) => {
    // Slash commands, prompts the user didn't type (including task notifications and
    // subagent hand-backs, which arrive unflagged), and prompts delivered into a
    // running turn are left alone.
    if (e.command || e.isMeta || e.turnId || isHarnessPrompt(e.text) || !(await read($, enabled)) || !(await isJevCode($))) {
      return next(e)
    }
    if (!(await shouldOrchestrate($, e.text))) {
      return next(e)
    }
    await update($, active, () => true)
    await update($, workers, () => NO_WORKERS)
    await showStatus($)
    return next({ ...e, context: [...(e.context ?? []), BRIEF] })
  }).catch(($, e, next) => next(e))

  // A worker the planner starts while orchestrating: count it as running.
  on('agent.spawn', async ($, e, next) => {
    const started = await next(e)
    try {
      const id = started && 'agentId' in started ? started.agentId : undefined
      if (id && (await read($, active))) {
        await update($, workers, w => ({ ...w, running: [...w.running, id] }))
        await showStatus($)
      }
    } catch {
      // Bookkeeping only: never fail a spawn over the status line.
    }
    return started
  })

  // A worker's turn carries its agentId; the main loop's carries none. Only the main
  // turn ending ends the orchestrated turn, so a worker finishing first can't clear it.
  on('turn.complete', async ($, e, next) => {
    try {
      if (e.agentId) {
        if ((await read($, workers)).running.includes(e.agentId)) {
          await update($, workers, w => ({ running: w.running.filter(x => x !== e.agentId), done: w.done + 1 }))
          if (!(await read($, active)) && (await read($, workers)).running.length === 0) {
            await update($, workers, () => NO_WORKERS)
          }
          await showStatus($)
        }
      } else if (await read($, active)) {
        await update($, active, () => false)
        // Background workers can outlive the turn: keep showing them until they finish.
        if ((await read($, workers)).running.length === 0) await update($, workers, () => NO_WORKERS)
        await showStatus($)
      }
    } catch {
      // Status only.
    }
    return next(e)
  })
}
