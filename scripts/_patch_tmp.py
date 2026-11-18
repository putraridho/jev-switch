def sub(path, old, new, count=1):
    s = open(path).read()
    assert old in s, (path, old)
    s = s.replace(old, new, count)
    open(path, 'w').write(s)

sub('src/lib.rs', 'pub mod guard;\n', 'pub mod guard;\npub mod guard_cmd;\n')
sub('src/commands.rs', 'use crate::mode::Mode;', 'use crate::guard_cmd::GuardCmd;\nuse crate::mode::Mode;')
sub('src/commands.rs', '    /// `/why`: why the router', '    /// `/budget`, `/privacy`, `/failover`, `/health`: what fences in and watches the router.\n    Guard(GuardCmd),\n    /// `/why`: why the router')
sub('src/commands.rs', '        "/cost" | "/usage" => Command::Cost,\n', '        "/cost" | "/usage" => Command::Cost,\n        "/budget" | "/privacy" | "/failover" | "/health" => {\n            GuardCmd::parse(head, &line[head.len()..]).map_or_else(Command::Invalid, Command::Guard)\n        }\n')
sub('src/repl.rs', '            Command::Cost => self.page_out(&self.cost_text()),\n', '            Command::Cost => self.page_out(&self.cost_text()),\n            Command::Guard(cmd) => {\n                let text = self.engine.run_guard_command(cmd);\n                self.page_out(&text);\n            }\n')
sub('src/uifmt/commands.rs', '    Cmd { name: "/config"', '''    Cmd { name: "/budget", args: "[usd <n> | tokens <n> | off]", group: Group::Info, about: "Show spend by model and the plan use; set a cap on paid models", example: "/budget usd 5", hint: "[usd <n>|tokens <n>|off]" },
    Cmd { name: "/privacy", args: "[audit | local-only on|off]", group: Group::Info, about: "Show the privacy rules, the audit trail, local-only mode", example: "/privacy audit", hint: "[audit|local-only on|off]" },
    Cmd { name: "/failover", args: "[max-attempts <n> | paid-fallback on|off]", group: Group::Info, about: "Show or change how a turn fails over to another model", example: "/failover paid-fallback off", hint: "[max-attempts <n>|paid-fallback on|off]" },
    Cmd { name: "/health", args: "", group: Group::Info, about: "Show how each provider did this session: answers, failures, speed", example: "", hint: "" },
    Cmd { name: "/config"''')
sub('src/turn.rs', '    /// Show assistant text the moment', '''    /// Run `/budget`, `/privacy`, `/failover` or `/health` against this session's guards.
    pub fn run_guard_command(&mut self, cmd: crate::guard_cmd::GuardCmd) -> String {
        let plans: Vec<(String, f64)> = crate::providers::ALL_PROVIDERS
            .into_iter()
            .filter_map(|p| self.runtime.plan_utilization(p).map(|u| (p.to_string(), u)))
            .collect();
        crate::guard_cmd::run(cmd, &mut self.guards, self.persist.as_deref(), &plans)
    }

    /// Show assistant text the moment''')
sub('src/turn.rs', '        self.last_context_tokens = out.usage.context_tokens.unwrap_or(0);\n', '        self.last_context_tokens = out.usage.context_tokens.unwrap_or(0);\n        self.guards.meter.record(spec.provider, spec.id, &out.usage, &self.guards.prices);\n')
