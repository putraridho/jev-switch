//! `/cache` and `/effort`: the cache dashboard and the reasoning-effort setting. Pure
//! wording lives in [`crate::turn::domain::cache`], [`crate::routing::domain::warm_cache`] and [`crate::turn::domain::effort`]; this maps them onto the session.

use crate::tui::repl::session::Repl;
use crate::backend::ProviderId;
use crate::routing::domain::warm_cache::{self, WarmCache};
use crate::shared::domain::wording::human_count;
use crate::tui::render::Style;
use crate::turn::domain::cache::{by_provider, per_turn, MissCause, TurnCache};
use crate::turn::domain::session_log::SessionLog;
use crate::catalog::domain::model_table as catalog;
use crate::turn::domain::effort;
use crate::turn::domain::effort::EffortSetting;

impl Repl {
    /// `/cache`.
    pub(super) fn cache_text(&self) -> String {
        dashboard(&self.log, self.engine.warm_cache().as_ref(), self.now(), self.out_style())
    }

    /// Providers with at least one model that can run now.
    fn ready_providers(&self) -> Vec<ProviderId> {
        let usable = self.engine.runtime.usable(self.now());
        crate::shared::ALL_PROVIDERS.into_iter().filter(|p| catalog::models().iter().any(|m| m.provider == *p && usable.contains(m))).collect()
    }

    /// `/effort` with no argument shows the setting; with one, sets it.
    pub(super) fn effort(&mut self, setting: Option<EffortSetting>) {
        if let Some(s) = setting {
            self.engine.set_effort(s);
        }
        let text = effort::effort_text(self.engine.effort(), &self.ready_providers());
        self.page_out(&text);
    }
}

/// `/cache`: the dashboard.
fn dashboard(log: &SessionLog, warm: Option<&WarmCache>, now_ms: u64, style: Style) -> String {
    let rows = by_provider(log);
    if rows.is_empty() {
        return format!("  {}", style.dim("No cache data yet: no turn in this session reported token usage."));
    }
    let mut lines = vec![style.bold("Cache by provider")];
    for r in &rows {
        lines.push(format!(
            "  {:<10}  hit {:>3}%  read {:>6}  rebuilt {:>6}  saved ~{} tokens of input cost",
            r.provider,
            r.hit_percent(),
            human_count(r.read),
            human_count(r.rebuilt),
            human_count(r.saved_tokens())
        ));
    }
    if let Some(w) = warm {
        let state = warm_cache::countdown(w, now_ms).unwrap_or_else(|| format!("{} has no documented cache window", w.provider));
        lines.push(String::new());
        lines.push(format!("  {} {} tokens cached; {state}", style.bold("Warm now"), human_count(w.tokens)));
        if let Some(warning) = warm_cache::expiry_warning(w, now_ms) {
            lines.push(format!("  {} {warning}", style.yellow("⚠")));
        }
    }
    let misses: Vec<TurnCache> = per_turn(log).into_iter().filter(|t| t.miss.is_some()).collect();
    if !misses.is_empty() {
        lines.push(String::new());
        lines.push(style.bold("Cache misses"));
        for m in misses.iter().rev().take(8).rev() {
            let why = m.miss.map_or("", MissCause::label);
            lines.push(format!("  turn {:<3} {}  hit {:>3}%  rebuilt {:>6}  {}", m.turn, m.model, m.hit_percent(), human_count(m.rebuilt), style.yellow(why)));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::Usage;
    use crate::turn::domain::session_log::TurnRecord;
    use crate::turn::domain::turn_status::TurnStatus;

    fn usage(input: u64, cached: u64) -> Usage {
        Usage { input_tokens: input, cached_input_tokens: cached, output_tokens: 1, context_tokens: None }
    }

    fn rec(provider: ProviderId, model: &str, input: u64, cached: u64, ended_ms: u64) -> TurnRecord {
        TurnRecord {
            prompt: String::new(),
            answer: "a".into(),
            status: TurnStatus::Answered,
            model: format!("{provider} · {model}"),
            route: Some((provider, model.into())),
            usage: Some(usage(input, cached)),
            elapsed_ms: 1000,
            ended_ms,
            tool_calls: 0,
        }
    }

    fn log_of(turns: Vec<TurnRecord>) -> SessionLog {
        let mut log = SessionLog::default();
        turns.into_iter().for_each(|t| log.push(t));
        log
    }

    #[test]
    fn dashboard_shows_providers_warmth_and_misses() {
        let log = log_of(vec![rec(ProviderId::Claude, "sonnet", 40_000, 0, 1_000), rec(ProviderId::Claude, "sonnet", 45_000, 40_000, 5_000)]);
        let w = WarmCache::after_turn(ProviderId::Claude, &usage(45_000, 40_000), 5_000);
        let text = dashboard(&log, Some(&w), 5_000 + 250_000, Style::off());
        assert!(text.contains("Cache by provider") && text.contains("hit  47%"), "{text}");
        assert!(text.contains("Warm now") && text.contains("expires in 50s"), "{text}");
        assert!(text.contains("Cache misses") && text.contains("turn 1") && text.contains("first turn"), "{text}");
        assert!(dashboard(&SessionLog::default(), None, 0, Style::off()).contains("No cache data yet"));
    }
}
