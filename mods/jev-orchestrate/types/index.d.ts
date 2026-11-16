export type Mode = 'on' | 'off'

declare module 'claude-code' {
  interface PluginState {
    'jev-orchestrate': { enabled: boolean; active: boolean }
  }
}
