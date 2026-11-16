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
