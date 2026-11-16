import { expect, test } from 'claude-code/testing'
import type { On } from 'claude-code'

import { isHarnessPrompt } from './harness'
import { BRIEF, parseArgs } from './register'

type Setup = {
  model?: string
  trigger?: boolean
  gatewayDown?: boolean
  /// The gateway never answers; the check's timer fires instead.
  gatewayHangs?: boolean
}

/// Stand in for the engine and the gateway; returns the requests the mod sent.
function engine(on: On, s: Setup = {}) {
  const sent: { url: string; body: string }[] = []
  on('command.register', () => ({ value: { command: 'orchestrate' } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('session.model', () => ({ value: s.model ?? 'jev-code[1m]' }))
  on('session.id', () => ({ value: 'sess-1' }))
  on('env.get', () => ({ value: 'http://127.0.0.1:18780' }))
  on('ui.status', () => ({ value: undefined }))
  // The check's timeout: never fires unless the gateway hangs.
  on('clock.sleep', () => (s.gatewayHangs ? { value: undefined } : new Promise<never>(() => {})))
  on('http.fetch', (_, e) => {
    if (s.gatewayDown) throw new Error('connection refused')
    if (s.gatewayHangs) return new Promise<never>(() => {})
    sent.push({ url: e.url, body: String(e.init?.body ?? '') })
    return { value: { status: 200, ok: true, headers: {}, text: JSON.stringify({ trigger: s.trigger ?? false }) } }
  })
  on('prompt.submit', (_, e) => ({ text: e.text, context: e.context }))
  return sent
}

/// A prompt as the person types it at an idle session.
const typed = (text: string) => ({ text, wait: false, origin: { kind: 'user' } }) as never

const BIG = 'add audit logging across the API, the web app and the tests'

test('a multi-part prompt gets the brief', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const r = await $.prompt.submit(typed(BIG))

  expect(r.context).toContain(BRIEF)
  expect(sent.length).toBe(1)
  expect(sent[0]?.url).toBe('http://127.0.0.1:18780/jev/orchestrate')
  expect(JSON.parse(sent[0]?.body ?? '{}')).toEqual({ prompt: BIG, session: 'sess-1' })
})

test('a prompt the gateway does not flag passes unchanged', async ($, on) => {
  engine(on, { trigger: false })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const r = await $.prompt.submit(typed('what does this flag do?'))

  expect(r.context ?? []).not.toContain(BRIEF)
})

test('other models are never orchestrated or checked', async ($, on) => {
  const sent = engine(on, { trigger: true, model: 'claude-sonnet-5-5' })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const r = await $.prompt.submit(typed(BIG))

  expect(r.context ?? []).not.toContain(BRIEF)
  expect(sent.length).toBe(0)
})

test('task notifications and hand-backs are never checked', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
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
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const r = await $.prompt.submit(typed(BIG))

  expect(r.text).toBe(BIG)
  expect(r.context ?? []).not.toContain(BRIEF)
})

test('a gateway that hangs times out and lets the prompt through', async ($, on) => {
  engine(on, { trigger: true, gatewayHangs: true })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const r = await $.prompt.submit(typed(BIG))

  expect(r.text).toBe(BIG)
  expect(r.context ?? []).not.toContain(BRIEF)
})

test('/orchestrate toggles, and on/off set', async ($, on) => {
  const sent = engine(on, { trigger: true })
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const run = (args: string) =>
    $.command.run({ command: 'orchestrate', args, origin: { kind: 'user' }, presentation: 'text' } as never)

  expect((await run('')).text).toContain('off')
  expect((await $.prompt.submit(typed(BIG))).context ?? []).not.toContain(BRIEF)
  expect(sent.length).toBe(0)

  expect((await run('off')).text).toContain('off')
  expect((await run('')).text).toContain('on')
  expect((await run('off')).text).toContain('off')
  expect((await run('on')).text).toContain('on')
  expect((await $.prompt.submit(typed(BIG))).context).toContain(BRIEF)

  expect((await run('maybe')).text).toContain('Usage')
})

test('argument parsing', () => {
  expect(parseArgs('')).toBe('toggle')
  expect(parseArgs(' ON ')).toBe('on')
  expect(parseArgs('off')).toBe('off')
  expect(parseArgs('later')).toBe('usage')
})
