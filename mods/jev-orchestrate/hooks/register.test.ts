import { expect, test } from 'claude-code/testing'
import type { On } from 'claude-code'

import { isHarnessPrompt } from './harness'
import { BRIEF, parseArgs, statusText } from './register'

type Setup = {
  model?: string
  trigger?: boolean
  gatewayDown?: boolean
  /// The gateway never answers; the check's timer fires instead.
  gatewayHangs?: boolean
  /// Collects every status line the mod sets.
  statuses?: unknown[]
  /// The value of JEV_ORCH_DEFAULT in the environment.
  envDefault?: string
  /// What the gateway's /jev/worker answers (default: no opinion).
  workerModel?: string | null
  workerRequests?: string[]
}

/// Stand in for the engine and the gateway; returns the requests the mod sent.
function engine(on: On, s: Setup = {}) {
  const sent: { url: string; body: string }[] = []
  on('command.register', () => ({ value: { command: 'orchestrate' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('session.model', () => ({ value: s.model ?? 'jev-code[1m]' }))
  on('session.id', () => ({ value: 'sess-1' }))
  on('env.get', (_, e) => ({ value: e.name === 'JEV_ORCH_DEFAULT' ? s.envDefault : 'http://127.0.0.1:18780' }))
  on('ui.status', (_, e) => (s.statuses?.push(e), { value: undefined }))
  // The check's timeout: never fires unless the gateway hangs.
  on('clock.sleep', () => (s.gatewayHangs ? { value: undefined } : new Promise<never>(() => {})))
  on('http.fetch', (_, e) => {
    if (s.gatewayDown) throw new Error('connection refused')
    if (s.gatewayHangs) return new Promise<never>(() => {})
    if (e.url.endsWith('/jev/worker')) {
      s.workerRequests?.push(String(e.init?.body ?? ''))
      return { value: { status: 200, ok: true, headers: {}, text: JSON.stringify({ model: s.workerModel ?? null }) } }
    }
    sent.push({ url: e.url, body: String(e.init?.body ?? '') })
    return { value: { status: 200, ok: true, headers: {}, text: JSON.stringify({ trigger: s.trigger ?? false }) } }
  })
  on('prompt.submit', (_, e) => ({ text: e.text, context: e.context }))
  return sent
}

/// A prompt as the person types it at an idle session.
const typed = (text: string) => ({ text, wait: false, origin: { kind: 'user' } }) as never

/// Starts a session and turns orchestrate mode on (it starts off).
async function start($: Parameters<Parameters<typeof test>[1]>[0]) {
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  await $.command.run({ command: 'orchestrate', args: 'on', origin: { kind: 'user' }, presentation: 'text' } as never)
}

const BIG = 'add audit logging across the API, the web app and the tests'

test('a multi-part prompt gets the brief', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await start($)
  const r = await $.prompt.submit(typed(BIG))

  expect(r.context).toContain(BRIEF)
  expect(sent.length).toBe(1)
  expect(sent[0]?.url).toBe('http://127.0.0.1:18780/jev/orchestrate')
  expect(JSON.parse(sent[0]?.body ?? '{}')).toEqual({ prompt: BIG, session: 'sess-1' })
})

test('a prompt the gateway does not flag passes unchanged', async ($, on) => {
  engine(on, { trigger: false })
  await start($)
  const r = await $.prompt.submit(typed('what does this flag do?'))

  expect(r.context ?? []).not.toContain(BRIEF)
})

test('other models are never orchestrated or checked', async ($, on) => {
  const sent = engine(on, { trigger: true, model: 'claude-sonnet-5-5' })
  await start($)
  const r = await $.prompt.submit(typed(BIG))

  expect(r.context ?? []).not.toContain(BRIEF)
  expect(sent.length).toBe(0)
})

test('task notifications and hand-backs are never checked', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await start($)
  const note = '<task-notification>\n<task-id>a1</task-id>\nadd audit logging across the API, the web app and the tests\n</task-notification>'
  const r = await $.prompt.submit(typed(note))

  expect(r.context ?? []).not.toContain(BRIEF)
  expect(sent.length).toBe(0)
})

test('harness prompts are recognised, typed ones are not', () => {
  expect(isHarnessPrompt('<task-notification>\n<task-id>a1</task-id>')).toBe(true)
  expect(isHarnessPrompt('  Another Claude session sent a message:\n<agent-message from="a">')).toBe(true)
  expect(isHarnessPrompt('Permission granted for this exact command only, as one whole string: "git status"')).toBe(true)
  expect(isHarnessPrompt('add audit logging across the API')).toBe(false)
  expect(isHarnessPrompt('why does the task-notification hook fire twice?')).toBe(false)
})

test('a gateway that is down lets the prompt through', async ($, on) => {
  engine(on, { gatewayDown: true })
  await start($)
  const r = await $.prompt.submit(typed(BIG))

  expect(r.text).toBe(BIG)
  expect(r.context ?? []).not.toContain(BRIEF)
})

test('a gateway that hangs times out and lets the prompt through', async ($, on) => {
  engine(on, { trigger: true, gatewayHangs: true })
  await start($)
  const r = await $.prompt.submit(typed(BIG))

  expect(r.text).toBe(BIG)
  expect(r.context ?? []).not.toContain(BRIEF)
})

test('it starts off: no check, no brief, until /orchestrate on', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const r = await $.prompt.submit(typed(BIG))

  expect(r.context ?? []).not.toContain(BRIEF)
  expect(sent.length).toBe(0)
})

test('JEV_ORCH_DEFAULT=on starts a session with orchestrate mode on', async ($, on) => {
  engine(on, { trigger: true, envDefault: 'on' })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  expect((await $.prompt.submit(typed(BIG))).context).toContain(BRIEF)
})

test('/orchestrate toggles, and on/off set', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const run = (args: string) =>
    $.command.run({ command: 'orchestrate', args, origin: { kind: 'user' }, presentation: 'text' } as never)

  expect((await run('')).text).toContain('on')
  expect((await run('')).text).toContain('off')
  expect((await $.prompt.submit(typed(BIG))).context ?? []).not.toContain(BRIEF)
  expect(sent.length).toBe(0)

  expect((await run('on')).text).toContain('on')
  expect((await run('off')).text).toContain('off')
  expect((await run('on')).text).toContain('on')
  expect((await $.prompt.submit(typed(BIG))).context).toContain(BRIEF)

  expect((await run('maybe')).text).toContain('Usage')
})

test('status line shows planning, running workers, then review', () => {
  expect(statusText(false, 0, 0)).toBeUndefined()
  expect(statusText(true, 0, 0)).toBe('orchestrating · planning')
  expect(statusText(true, 3, 0)).toBe('orchestrating · 3 workers running, 0 done')
  expect(statusText(true, 1, 2)).toBe('orchestrating · 1 worker running, 2 done')
  expect(statusText(true, 0, 3)).toBe('orchestrating · 3 done, reviewing')
  // The turn ended but a background worker is still going.
  expect(statusText(false, 1, 2)).toBe('orchestrating · 1 worker running, 2 done')
})

test('workers are counted, and a worker finishing does not end the orchestrated turn', async ($, on) => {
  const statuses: unknown[] = []
  engine(on, { trigger: true, statuses })
  let n = 0
  on('agent.spawn', () => ({ model: 'sonnet', agentId: `w${++n}` }))
  on('turn.complete', () => ({ text: '' }))
  await start($)
  await $.prompt.submit(typed(BIG))
  await $.agent.spawn({ prompt: 'step 1' } as never)
  await $.agent.spawn({ prompt: 'step 2' } as never)
  const seen = () => JSON.stringify(statuses)
  expect(seen()).toContain('2 workers running, 0 done')

  await $.turn.complete({ agentId: 'w1', answer: 'ok', durationMs: 1, isAborted: false, turnId: 't1', reason: 'answer' } as never)
  expect(seen()).toContain('1 worker running, 1 done')
  expect(JSON.stringify(statuses.at(-1))).not.toContain('orchestrate: on')

  await $.turn.complete({ agentId: 'w2', answer: 'ok', durationMs: 1, isAborted: false, turnId: 't2', reason: 'answer' } as never)
  expect(seen()).toContain('2 done, reviewing')

  await $.turn.complete({ answer: 'done', durationMs: 1, isAborted: false, turnId: 't0', reason: 'answer' } as never)
  expect(JSON.stringify(statuses.at(-1))).toContain('orchestrate: on')
})

test('Jev picks each worker\'s model while orchestrating, and only then', async ($, on) => {
  const asked: string[] = []
  engine(on, { trigger: true, workerModel: 'haiku', workerRequests: asked })
  const received: (string | undefined)[] = []
  on('agent.spawn', (_, e) => (received.push(e.model), { model: e.model ?? 'sonnet', agentId: `w${received.length}` }))
  on('turn.complete', () => ({ text: '' }))
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })

  // Orchestration off: the planner's choice is untouched and the gateway is not asked.
  await $.agent.spawn({ prompt: 'look around', model: 'sonnet' } as never)
  expect(received).toEqual(['sonnet'])
  expect(asked.length).toBe(0)

  await start($)
  await $.prompt.submit(typed(BIG))
  await $.agent.spawn({ prompt: 'harden the session cookie handling', model: 'sonnet' } as never)
  expect(received.at(-1)).toBe('haiku')
  expect(JSON.parse(asked[0] ?? '{}')).toEqual({ prompt: 'harden the session cookie handling', session: 'sess-1' })
})

test('the gateway may give a worker Opus (under a Fable planner), never Fable', async ($, on) => {
  const received: (string | undefined)[] = []
  engine(on, { trigger: true, workerModel: 'opus' })
  on('agent.spawn', (_, e) => (received.push(e.model), { model: e.model ?? 'sonnet', agentId: `w${received.length}` }))
  await start($)
  await $.prompt.submit(typed(BIG))
  await $.agent.spawn({ prompt: 'harden the session cookie handling', model: 'sonnet' } as never)
  expect(received).toEqual(['opus'])
})

test('an answer that is not haiku, sonnet or opus is ignored', async ($, on) => {
  const received: (string | undefined)[] = []
  engine(on, { trigger: true, workerModel: 'claude-fable-5-2' })
  on('agent.spawn', (_, e) => (received.push(e.model), { model: e.model ?? 'sonnet', agentId: 'w1' }))
  await start($)
  await $.prompt.submit(typed(BIG))
  await $.agent.spawn({ prompt: 'redesign the ledger', model: 'sonnet' } as never)
  expect(received).toEqual(['sonnet'])
})

test('with no answer from Jev the planner\'s model stands', async ($, on) => {
  const received: (string | undefined)[] = []
  engine(on, { trigger: true, workerModel: null })
  on('agent.spawn', (_, e) => (received.push(e.model), { model: e.model ?? 'sonnet', agentId: 'w1' }))
  await start($)
  await $.prompt.submit(typed(BIG))
  await $.agent.spawn({ prompt: 'rename the field', model: 'sonnet' } as never)
  expect(received).toEqual(['sonnet'])
})

test('argument parsing', () => {
  expect(parseArgs('')).toBe('toggle')
  expect(parseArgs(' ON ')).toBe('on')
  expect(parseArgs('off')).toBe('off')
  expect(parseArgs('later')).toBe('usage')
})
