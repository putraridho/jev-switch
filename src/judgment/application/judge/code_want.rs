//! Code-mode routing: which tier a turn needs, from Jev's signals and the security keywords.

use serde_json::{json, Value};

use crate::judgment::domain::security::{security_level, Security};
use crate::judgment::domain::thresholds::{CODE_EDIT_P, CODE_SECURITY_P};
use crate::judgment::domain::want::{worker_alias, CodeWant};

use super::Router;

impl Router {
    /// Which model an orchestrated worker's task needs (Sonnet, or Haiku for a read-only
    /// lookup when enabled), judged by Jev from the worker's own prompt with the same
    /// signals as code mode (security review, reasoning, edits) but not its keyword lists.
    /// `None` when Jev is unavailable: the planner's own choice then stands.
    pub async fn worker_want(&self, task: &str, allow_opus: bool) -> Option<(&'static str, String, Value)> {
        let want = self.code_want_judged(task, "", 1, false).await;
        if want.signals.is_null() {
            return None;
        }
        let alias = worker_alias(want.tier, &want.signals, self.worker_cheap, allow_opus);
        Some((alias, want.reason, want.signals.to_json()))
    }


    /// Code-mode tier for one turn, before session stickiness: 1 mid or 2 strong.
    /// Code mode never uses the cheap model: across five repos Haiku explanations
    /// held up ~85% at every Jev confidence, so no cutoff could sort out its misses,
    /// and per-model prompt caches make its savings small in a coding session.
    /// Strong security signals, security questions without keywords (Jev
    /// `security_review`), confident reasoning, and edits to auth code → strong;
    /// everything else → mid. If Jev is unavailable, returns `fallback`.
    pub async fn code_want(&self, request: &str, recent_context: &str, fallback: usize) -> CodeWant {
        let mut want = self.code_want_base(request, recent_context, fallback).await;
        let Some(cutoff) = self.top_threshold else { return want };
        if want.tier != 2 {
            return want;
        }
        // Only Opus-bound turns pay for the extra judgment. If Jev fails or is slow, stay on Opus.
        if let Ok(Ok(p)) = tokio::time::timeout(self.jev_timeout, self.very_hard(request, recent_context)).await {
            let mut signals = want.signals.to_json();
            match &mut signals {
                Value::Object(o) => {
                    o.insert("hard_p".into(), json!(p));
                }
                s => *s = json!({ "hard_p": p }),
            }
            want.signals = signals.into();
            if p >= cutoff {
                want.tier = 3;
                want.reason = format!("very hard (hard p {p:.2}): {}", want.reason);
            }
        }
        want
    }

    async fn code_want_base(&self, request: &str, recent_context: &str, fallback: usize) -> CodeWant {
        self.code_want_judged(request, recent_context, fallback, true).await
    }

    /// `keywords` false ignores the security keyword lists and relies on Jev's own
    /// judgments. An orchestrated worker's task is a long brief the planner wrote, full
    /// of incidental words ("no unsafe code", "don't leak the lock", "audit"): on the
    /// three workers of a concurrency redesign the keyword rule tagged every one
    /// security-sensitive, which under a Fable planner means three Opus workers.
    async fn code_want_judged(&self, request: &str, recent_context: &str, fallback: usize, keywords: bool) -> CodeWant {
        let security = if keywords { security_level(request) } else { Security::None };
        if security == Security::Strong {
            return CodeWant {
                tier: 2,
                confident: true,
                reason: "security-sensitive".into(),
                signals: json!({ "security": "strong" }).into(),
            };
        }
        let unavailable = |why: String| CodeWant {
            tier: fallback,
            confident: false,
            reason: format!("jev unavailable ({why})"),
            signals: Value::Null.into(),
        };
        let judged = tokio::time::timeout(self.jev_timeout, async {
            tokio::join!(
                self.classify(request, recent_context),
                self.needs_changes(request, recent_context),
                self.security_review(request, recent_context)
            )
        })
        .await;
        let (ans, edit_p, sec_p) = match judged {
            Ok((Ok(a), Ok(p), Ok(s))) => (a, p, s),
            Ok((Err(e), _, _)) | Ok((_, Err(e), _)) | Ok((_, _, Err(e))) => return unavailable(format!("{e:#}")),
            Err(_) => return unavailable(format!("timed out after {} ms", self.jev_timeout.as_millis())),
        };
        let tag = format!("{} {:.2}, edit p {:.2}, sec p {:.2}", ans.choice, ans.confidence, edit_p, sec_p);
        let signals = json!({
            "choice": ans.choice, "confidence": ans.confidence, "edit_p": edit_p, "sec_p": sec_p,
            "security": format!("{security:?}").to_lowercase(),
        }).into();
        if sec_p >= CODE_SECURITY_P {
            return CodeWant { tier: 2, confident: true, reason: format!("security review ({tag})"), signals };
        }
        if ans.choice == "reasoning" && ans.confidence >= self.strong_threshold {
            return CodeWant { tier: 2, confident: true, reason: format!("confidently reasoning ({tag})"), signals };
        }
        if security == Security::Weak && edit_p >= CODE_EDIT_P {
            return CodeWant { tier: 2, confident: true, reason: format!("changes auth/session code ({tag})"), signals };
        }
        // Confident it isn't hard work: lets a session step back down from strong.
        // Short replies ("thanks", "done") come back `unclear` but aren't edits.
        let confident = (matches!(ans.choice.as_str(), "simple" | "standard") && ans.confidence >= 0.5)
            || (ans.choice == "unclear" && edit_p < 0.2);
        CodeWant { tier: 1, confident, reason: format!("everyday ({tag})"), signals }
    }
}
