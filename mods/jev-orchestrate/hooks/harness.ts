// Prompts Claude Code puts in the user slot that the person never typed. They are
// never checked for orchestration: a check costs a Jev call and the prompt waits for
// it, and a subagent's report could score high and start a plan nobody asked for.
const PREFIXES = [
  '<task-notification',
  '<agent-message',
  'Another Claude session sent a message:',
  'Permission granted for this exact command only',
]

export function isHarnessPrompt(text: string): boolean {
  const t = text.trimStart()
  return PREFIXES.some(p => t.startsWith(p))
}

type Block = { type: string; text?: unknown }

/// What the last answer said, for the orchestrate check. A prompt like "do 1 and run the
/// review" is only multi-part in light of the list the answer before it gave, and the
/// check sees the prompt alone otherwise. Mirrors the gateway's own routing context
/// ("assistant: ..."), with more room, so the check and the router read the same thing.
export function recentContext(messages: readonly { role: string; content: readonly Block[] }[], max = 600): string {
  const last = [...messages].reverse().find(m => m.role === 'assistant')
  if (!last) return ''
  const text = last.content
    .filter(b => b.type === 'text' && typeof b.text === 'string')
    .map(b => (b.text as string).replace(/<system-reminder>[\s\S]*?(<\/system-reminder>|$)/g, ''))
    .join('\n')
    .trim()
  return text ? `assistant: ${text.slice(0, max)}` : ''
}
