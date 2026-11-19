//! The router: Jev judges the *task* (not the model name), code maps it to one of
//! three tiers, and confidence gates the cheap route:
//!   simple (confident) → Haiku · reasoning (confident) → Opus · everything else → Sonnet.
//! Both cutoffs are calibrated, not guessed (see docs/guide.md).

use anyhow::Result;
use serde_json::{json, Value};

use crate::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::catalog::TaskType;
use crate::typesafe::{choice_question, decode_choice, decode_noul, noul_question, ChoiceAnswer, TypeSafe};

#[derive(Debug)]
pub struct Decision {
    pub model: &'static str,
    pub kind: String,
    pub confidence: f64,
    pub reason: &'static str,
}

/// A code-mode routing judgment for one turn (see [`Router::code_want`]).
pub struct CodeWant {
    pub tier: usize,
    pub confident: bool,
    pub reason: String,
    /// Jev's raw signals for the usage log (`Value::Null` when Jev wasn't asked).
    pub signals: Value,
}

/// Jev's reading of one turn for provider routing (see [`Router::provider_want`]).
/// Jev only reports; mapping a task type to models is the catalog's job.
#[derive(Debug, Clone)]
pub struct ProviderWant {
    pub task: TaskType,
    /// Jev's confidence in `task`.
    pub task_confidence: f64,
    /// P(only the most capable model will do).
    pub very_hard_p: f64,
    /// P(the request is about security), independent of keywords.
    pub security_p: f64,
    /// P(the request modifies files or runs commands).
    pub edit_p: f64,
    /// P(the message continues the task in progress rather than starting a new one).
    pub continues_p: f64,
    /// P(the message reacts to the previous answer as wrong); `None` with no previous turn.
    pub correction_p: Option<f64>,
    /// Jev's orchestration judgment: P(the request is a large task that splits into separate
    /// pieces), the signal orchestration triggers on (`ORCH_TRIGGER_P`), and the sequence
    /// veto. Asked in the same single batch.
    pub orchestration: Orchestration,
    /// What the keyword lists say, kept alongside Jev's judgment.
    pub security_keywords: Security,
    /// Every number above plus the task name, for the decision log.
    pub signals: Value,
}

/// `needs_changes` probability at or above which a code turn counts as editing.
pub const CODE_EDIT_P: f64 = 0.5;
/// Code mode: `security_review` probability at or above which a turn goes strong
/// even with no security keyword ("can a viewer see X?", "who can call this?").
/// On lunagraph (2,296 prompts, 408 Sonnet answers graded against the code), the
/// keyword-less security questions Sonnet missed and Opus fixed scored 0.86-0.97;
/// 0.85 catches all but one at 0.60, for 0.9% more turns on Opus than 0.90.
pub const CODE_SECURITY_P: f64 = 0.85;
/// Code mode default for `CODE_STRONG_THRESHOLD`: Jev `reasoning` confidence at or
/// above which a turn goes strong. On lunagraph, Sonnet's answers with reasoning
/// confidence 0.75-0.85 were 31 correct and 2 partial that Opus didn't fix either;
/// its misses Opus fixed scored 0.89 and 0.92. Raising it from 0.75 cut Opus turns
/// from 32% to 28% with no fixable miss lost.
pub const CODE_STRONG_THRESHOLD: f64 = 0.85;

/// Orchestrate mode default for `ORCH_TRIGGER_P`: `multi_part` probability at or above
/// which a prompt is orchestrated. Scored on 1,551 distinct logged prompts (2026-11-16,
/// `orch_tune`): 12 (0.8%) scored 0.70 or more, and all but one short follow-up were
/// real multi-part work (rebuild an app, 14 tickets, 20 page variants, four issues in
/// one PR). The 0.60-0.70 band starts mixing in follow-ups and single asks.
pub const ORCH_TRIGGER_P: f64 = 0.70;

/// `sequential` probability at or above which a multi-part request is NOT orchestrated:
/// steps that must run in order ("run the tests, then check the report, then refactor",
/// "do 1 -> 2 -> 3") gain nothing from parallel workers. A veto, not a rescaling, so
/// the calibrated `multi_part` scores of genuinely parallel work are untouched.
pub const SEQUENTIAL_VETO_P: f64 = 0.70;

/// Jev's judgment of whether a request should be orchestrated: the two raw answers and the
/// veto they imply. The veto is a decision of its own, not a lowered score: a cutoff set
/// at or under any cap would let a sequence through, so the gate ([`crate::orchestrate::orch_gate`])
/// refuses a vetoed request whatever the cutoff is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Orchestration {
    /// Jev's `multi_part` answer: what the cutoff is compared with.
    pub multi_part_p: f64,
    /// Jev's `sequential` answer; `None` when it was not asked (orchestration is off) or
    /// could not be read, which leaves no veto information: only `multi_part` decides.
    pub sequential_p: Option<f64>,
    /// The request is a sequence of dependent steps (`sequential_p` at [`SEQUENTIAL_VETO_P`]
    /// or more): parallel workers gain nothing, so it is never orchestrated.
    pub vetoed: bool,
}

impl Orchestration {
    pub fn new(multi_part_p: f64, sequential_p: f64) -> Self {
        Self::judged(multi_part_p, Some(sequential_p))
    }

    /// The judgment from the answers there are: without a `sequential` answer nothing is vetoed.
    pub fn judged(multi_part_p: f64, sequential_p: Option<f64>) -> Self {
        Self { multi_part_p, sequential_p, vetoed: sequential_p.is_some_and(|p| p >= SEQUENTIAL_VETO_P) }
    }

    /// The fields of an orchestrate check's log row and response; all null when Jev gave
    /// no answer (`None`). The raw probabilities and the veto are all logged, so a report can
    /// say what a cutoff would have triggered and what the veto held back.
    pub fn fields(judgment: Option<&Self>) -> Value {
        json!({
            "multi_part_p": judgment.map(|j| j.multi_part_p),
            "sequential_p": judgment.map(|j| j.sequential_p),
            "vetoed": judgment.map(|j| j.vetoed),
        })
    }
}

/// Default of the gateway's `FABLE_THRESHOLD` environment variable (the one name users
/// set; this constant says "top" because it is the top tier, whatever model fills it):
/// `very_hard` probability at or above which a turn that would go to Opus goes to
/// Fable instead. `jev-code` routing uses the same value, not configurable. NOT
/// calibrated: there is no graded Opus-versus-Fable comparison yet, and Fable's price is
/// unknown. The cutoff is high so few turns qualify; the gateway logs `hard_p` on every Opus-bound turn so it can
/// be tuned from real usage. `JEV_FABLE=off` disables the tier.
pub const TOP_THRESHOLD: f64 = 0.90;

/// Most consecutive easier turns (confident or not) a tier is held for. A cache
/// rebuild costs about one turn's worth of re-reading the context at the new model's
/// write price; against a ~5x Opus premium that is repaid in ~3 turns, so holding
/// Opus through 4+ vague turns costs more than it saves. On the logged turns, 5 of
/// 24 holds ran 4-9 turns, all of them unclear/low-confidence ones that never counted
/// toward `downgrade_after`.
pub const MAX_HOLD: u32 = 4;

/// Default `JEV_TIMEOUT_MS`: the longest a routing judgment may take before the turn
/// goes ahead on the current model. A judgment normally takes ~0.5 s (three questions
/// in parallel); without a limit a stalled connection held the turn until it broke.
pub const JEV_TIMEOUT_MS: u64 = 3000;

/// Session stickiness for code-mode tiers. Prompt caches are per model, so a
/// switch rebuilds the whole context cache: upgrade at once, but step down (one
/// tier, never below mid) only after `downgrade_after` confident easier turns.
#[derive(Debug, Default, Clone)]
pub struct Sticky {
    /// Tier of the model that answered the previous turn.
    pub tier: Option<usize>,
    lower_streak: u32,
    /// Consecutive turns wanting a lower tier than `tier`, confident or not.
    held: u32,
}

impl Sticky {
    /// The tier for this turn given Jev's `want`. Callers set `tier` once the
    /// turn runs on it.
    pub fn decide(&mut self, w: &CodeWant, downgrade_after: u32) -> (usize, String) {
        let Some(cur) = self.tier else {
            return (w.tier, w.reason.clone());
        };
        if w.tier > cur {
            self.lower_streak = 0;
            self.held = 0;
            return (w.tier, format!("upgrade: {}", w.reason));
        }
        if w.tier == cur {
            self.lower_streak = 0;
            self.held = 0;
            return (cur, format!("stay (warm cache): {}", w.reason));
        }
        self.held += 1;
        if self.held >= MAX_HOLD && cur > 1 {
            self.lower_streak = 0;
            self.held = 0;
            return (cur - 1, format!("downgrade after holding {MAX_HOLD} easier turns: {}", w.reason));
        }
        if !w.confident {
            // Uncertain turn: keep the streak so one vague message doesn't erase progress.
            return (cur, format!("stay (warm cache): {}", w.reason));
        }
        self.lower_streak += 1;
        if self.lower_streak >= downgrade_after {
            self.lower_streak = 0;
            self.held = 0;
            return ((cur - 1).max(1), format!("downgrade after {downgrade_after} easier turns: {}", w.reason));
        }
        (cur, format!("stay ({}/{} toward downgrade): {}", self.lower_streak, downgrade_after, w.reason))
    }
}

pub struct Router {
    pub ts: TypeSafe,
    /// Route to the cheap model only when the task is `simple` AND confidence is
    /// at least this. Do NOT guess this — read it off the `tune` sweep.
    pub cheap_threshold: f64,
    /// Route `reasoning` to the strong model only at or above this confidence;
    /// below it, the mid model. 0.0 sends all `reasoning` to strong.
    pub strong_threshold: f64,
    /// When set, a turn headed to the strong model is also asked `very_hard`, and goes
    /// to tier 3 (Fable) at or above this. `None` keeps code mode at tiers 0-2.
    pub top_threshold: Option<f64>,
    /// Orchestrated workers: a read-only `simple` task at this confidence or above gets
    /// Haiku. `None` (the default) keeps every worker on Sonnet: Haiku is
    /// uncalibrated for workers, and a wrong report from one misleads the planner.
    pub worker_cheap: Option<f64>,
    /// Longest wait for one routing judgment; past it the turn keeps its current model.
    pub jev_timeout: std::time::Duration,
}

/// The model a worker should run on, as the alias `agent.spawn` takes. Fable is never a
/// worker. Opus may be one only when Fable is the planner (`allow_opus`), for a task Jev
/// judges security-sensitive or hard; under an Opus planner it stays on Sonnet and the
/// planner's own review checks it.
pub fn worker_alias(tier: usize, signals: &Value, cheap: Option<f64>, allow_opus: bool) -> &'static str {
    if tier >= 2 {
        return if allow_opus { "opus" } else { "sonnet" };
    }
    let edit_p = signals["edit_p"].as_f64().unwrap_or(1.0);
    let confidence = signals["confidence"].as_f64().unwrap_or(0.0);
    match cheap {
        Some(c) if signals["choice"] == "simple" && confidence >= c && edit_p < 0.2 => "haiku",
        _ => "sonnet",
    }
}

impl Router {
    pub fn new(ts: TypeSafe, cheap_threshold: f64) -> Self {
        let ms = crate::envfile::lookup("JEV_TIMEOUT_MS").and_then(|v| v.parse().ok()).unwrap_or(JEV_TIMEOUT_MS);
        Self {
            ts,
            cheap_threshold,
            strong_threshold: 0.0,
            top_threshold: None,
            worker_cheap: None,
            jev_timeout: std::time::Duration::from_millis(ms),
        }
    }

    pub fn with_jev_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.jev_timeout = timeout;
        self
    }

    pub fn with_worker_cheap(mut self, worker_cheap: Option<f64>) -> Self {
        self.worker_cheap = worker_cheap;
        self
    }

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
        Some((alias, want.reason, want.signals))
    }

    pub fn with_top_threshold(mut self, top_threshold: Option<f64>) -> Self {
        self.top_threshold = top_threshold;
        self
    }

    pub fn with_strong_threshold(mut self, strong_threshold: f64) -> Self {
        self.strong_threshold = strong_threshold;
        self
    }

    /// The single routing judgment, shared by `route` (production) and `tune`
    /// (calibration) so both evaluate the exact same question.
    pub async fn classify(&self, request: &str, recent_context: &str) -> Result<ChoiceAnswer> {
        self.ts
            .choice(
                json!({ "request": request, "recent_context": recent_context }),
                "What does this coding request need from the model?",
                json!({
                    "simple": {
                        "what": "a direct lookup, formatting, a short edit, a factual answer, or a small self-contained snippet: a one-liner, regex, command, config file, or a short standard function (under ~20 lines) with no multi-step reasoning",
                        "not_for": "anything needing planning, debugging across files, or tradeoff analysis; or any security-sensitive code (auth, crypto, signatures, passwords, tokens, input sanitization, injection)",
                        "examples": ["rename a variable", "what does this flag do", "format this JSON", "write a one-line regex", "check if a string is a palindrome", "write a Dockerfile for a Node app", "find the largest file in a directory"]
                    },
                    "standard": {
                        "what": "everyday coding that needs more than a short snippet but stays within one file or component: a module, class, script, middleware, CLI, tests, a small refactor, or debugging a specific error in given code",
                        "not_for": "one-liners, lookups, or short standard functions (simple), or work spanning many files or systems, architecture, or tradeoffs (reasoning)",
                        "examples": ["write a CLI that parses a CSV and prints a column average", "add pagination to this endpoint", "why does this async function return undefined", "write unit tests for this function"]
                    },
                    "reasoning": {
                        "what": "multi-step problem solving, system design, planning, debugging across files or systems, math or logic, or tradeoff analysis",
                        "examples": ["why does the upload hang on large files", "design the retry and backoff strategy", "refactor this module to remove the cycle"]
                    },
                    "unclear": {
                        "what": "ambiguous, mixed, or under-specified; not clearly simple, standard, or reasoning"
                    }
                }),
            )
            .await
    }

    /// A-priori routing from the request text (+ recent context), for a request
    /// with no prior turn to follow.
    pub async fn route(&self, request: &str, recent_context: &str) -> Result<Decision> {
        self.route_turn(request, recent_context, None).await
    }

    /// Routing for a turn in a conversation. `prev_model` is the model that
    /// answered the previous turn, if any. Precedence:
    /// 1. security-sensitive requests always go strong (hard rule, not Jev's call);
    /// 2. a short follow-up stays on the previous model unless Jev is confident
    ///    about a tier (confidently simple or confidently reasoning);
    /// 3. otherwise Jev's judgment: confidently simple → cheap; confidently
    ///    reasoning → strong; everything else (unsure simple/reasoning, standard,
    ///    unclear) → mid.
    pub async fn route_turn(
        &self,
        request: &str,
        recent_context: &str,
        prev_model: Option<&'static str>,
    ) -> Result<Decision> {
        let ans = self.classify(request, recent_context).await?;
        let go_cheap = ans.choice == "simple" && ans.confidence >= self.cheap_threshold;
        let go_strong = ans.choice == "reasoning" && ans.confidence >= self.strong_threshold;
        let (model, reason) = if is_security_sensitive(request) {
            (MODEL_STRONG, "security-sensitive \u{2192} strong")
        } else if let Some(prev) = prev_model.filter(|_| {
            !go_cheap && !go_strong && is_short_follow_up(request)
        }) {
            (prev, "short follow-up \u{2192} same model as last turn")
        } else if go_cheap {
            (MODEL_CHEAP, "confidently simple")
        } else if go_strong {
            (MODEL_STRONG, "confidently reasoning \u{2192} strong")
        } else {
            (MODEL_MID, "standard / unclear / low-confidence \u{2192} mid")
        };
        Ok(Decision { model, kind: ans.choice, confidence: ans.confidence, reason })
    }

    /// Code mode: probability that fulfilling `request` means modifying files or
    /// running commands (vs. only reading code and answering). Changing auth or
    /// session code goes to the strong model; reading it stays on mid.
    pub async fn needs_changes(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.ts.noul_question(request_state(request, recent_context), needs_changes_question()).await
    }

    /// Code mode: probability that `request` is about security: who can access or
    /// change what, or whether something is safe against misuse. Catches authz and
    /// trust-boundary questions that carry no security keyword, and lets keyword
    /// matches that are about something else (design tokens, React keys) through.
    pub async fn security_review(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.ts.noul_question(request_state(request, recent_context), security_review_question()).await
    }

    /// Probability that `request` is among the hardest work: it needs the most capable
    /// model, not just a strong one. Asked only of turns already headed to Opus.
    pub async fn very_hard(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.ts.noul_question(request_state(request, recent_context), very_hard_question()).await
    }

    /// Orchestrate mode's judgment, from ONE Jev request asking both `multi_part` (is
    /// `request` a large task that splits into separate pieces of work, so a planner can
    /// hand them to cheaper workers; questions, single fixes and short follow-ups score
    /// low) and `sequential` (see [`SEQUENTIAL_VETO_P`]). The only way to ask
    /// `multi_part` for orchestration, so no caller can skip the sequence veto.
    ///
    /// An unreadable `sequential` answer does not fail the judgment: it only loses the veto
    /// (`sequential_p` is `None`), the same as in [`Router::provider_want`]; the caller logs it.
    pub async fn orchestration_p(&self, request: &str, recent_context: &str) -> Result<Orchestration> {
        let questions = json!({ Q_MULTI_PART: multi_part_question(), Q_SEQUENTIAL: sequential_question() });
        let answers = self.ts.ask_many(request_state(request, recent_context), questions).await?;
        Ok(Orchestration::judged(decode_noul(&answers, Q_MULTI_PART)?, decode_noul(&answers, Q_SEQUENTIAL).ok()))
    }

    /// Outcome signal: probability that `request` reacts to the previous answer as
    /// wrong, incomplete or not what was wanted (a correction, complaint, retry or
    /// the same question again). Logged for later analysis; never used for routing.
    pub async fn is_correction(&self, request: &str, previous_request: &str, previous_answer: &str) -> Result<f64> {
        let state = json!({ "request": request, "previous_request": previous_request, "previous_answer": previous_answer });
        self.ts.noul_question(state, is_correction_question()).await
    }

    /// The whole provider-routing judgment in ONE request: which kind of task this is,
    /// plus the yes/no signals the router weighs it with. A single round trip (~0.5 s)
    /// instead of five keeps the added latency off every turn. `previous` is the prior
    /// (request, answer) pair; without it the correction question is not asked.
    /// `orchestrate`: orchestration is on, so the `sequential` veto question is asked too
    /// (it is used for nothing else). Bounded
    /// by `jev_timeout`; `None` on a timeout, any error or an unreadable answer, and the
    /// caller then stays on its current model.
    pub async fn provider_want(
        &self,
        request: &str,
        recent_context: &str,
        previous: Option<(&str, &str)>,
        orchestrate: bool,
    ) -> Option<ProviderWant> {
        let state = provider_state(request, recent_context, previous);
        let questions = provider_questions(previous.is_some(), orchestrate);
        let answers = tokio::time::timeout(self.jev_timeout, self.ts.ask_many(state, questions)).await.ok()?.ok()?;
        decode_provider_want(&answers, request, previous.is_some(), orchestrate).ok()
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
            match &mut want.signals {
                Value::Object(o) => {
                    o.insert("hard_p".into(), json!(p));
                }
                s => *s = json!({ "hard_p": p }),
            }
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
                signals: json!({ "security": "strong" }),
            };
        }
        let unavailable = |why: String| CodeWant {
            tier: fallback,
            confident: false,
            reason: format!("jev unavailable ({why})"),
            signals: Value::Null,
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
        });
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

    /// Verify variant: judge whether a cheap answer actually resolved the request.
    /// Returns true when adequate. Gate its probability on a threshold you also
    /// calibrate; this tracks outcome quality far more stably than a-priori
    /// difficulty, so it is the recommended escalation signal.
    pub async fn answer_is_adequate(
        &self,
        request: &str,
        answer: &str,
        threshold: f64,
    ) -> Result<(bool, f64)> {
        let p = self
            .ts
            .noul(
                json!({ "request": request, "answer": answer }),
                "Does the answer fully and correctly address the request?",
                json!({
                    "true": "complete, correct, and directly responsive",
                    "false": "incomplete, wrong, evasive, or only partially addresses it"
                }),
            )
            .await?;
        Ok((p >= threshold, p))
    }
}

// ---- Question definitions -------------------------------------------------------
// One definition per yes/no question, used by the single-call method AND the batch in
// `provider_want`, so the two can never drift apart (a change here changes both).

/// The state every code-mode question reads.
fn request_state(request: &str, recent_context: &str) -> Value {
    json!({ "request": request, "recent_context": recent_context })
}

fn needs_changes_question() -> Value {
    noul_question(
        "Does fulfilling this request require modifying files or running commands in the project?",
        json!({
            "true": "editing, creating, or deleting files; running builds, tests, scripts, git, or other commands",
            "false": "only reading or searching code and answering: explaining, locating, reviewing without changes"
        }),
    )
}

fn security_review_question() -> Value {
    noul_question(
        "Is this request about security: who can access or change what, or whether something is safe against misuse or attack?",
        json!({
            "true": "authorization and access control (can a user, role, or member see or change something they shouldn't), data exposure, trust boundaries, untrusted input (injection, path traversal, SSRF, XSS, unsafe URLs or IPC), secrets and credentials, crypto, signatures, or reviewing code for vulnerabilities",
            "false": "ordinary coding, debugging, or design; or security-sounding words that mean something else here: design or CSS tokens, React keys, editor or canvas session state, hashing for ids, caching or dedupe, sandboxed iframe layout"
        }),
    )
}

fn very_hard_question() -> Value {
    noul_question(
        "Is this one of the hardest requests, where only the most capable model will do and a mistake would be costly?",
        json!({
            "true": "designing or untangling a subtle architecture that spans many systems, debugging a deep concurrency, consistency or distributed-state problem, a security-critical design where a flaw would be costly, or reasoning that must stay exact across a very large amount of code",
            "false": "a question or explanation, a lookup, a routine feature, bug fix or refactor, reviewing a change, or anything a strong model would handle comfortably"
        }),
    )
}

fn multi_part_question() -> Value {
    noul_question(
        "Is this request a large piece of work that splits into several separate steps, each changing a different part of the codebase?",
        json!({
            "true": "building or changing a feature across several files, layers or systems (API, UI, database, tests, docs), a migration or refactor touching many modules, or a list of distinct changes to make; each piece could be done and checked on its own",
            "false": "a question or explanation, a lookup, reviewing or investigating without a set of changes, a single bug fix or small edit in one place, a one-line command, or a short reply or follow-up to the conversation"
        }),
    )
}

fn sequential_question() -> Value {
    noul_question(
        "Is this request a sequence of steps that must be done in order, each depending on the result of the previous one, rather than independent pieces that could be worked on at the same time?",
        json!({
            "true": "steps in a fixed order where a later step needs an earlier one's result: run the tests first, then check the report, then refactor; address one PR's comments, then the next stacked PR; or a short go-ahead for steps already discussed, such as \"do 1 -> 2 -> 3\" or \"go with the plan in that order\"",
            "false": "independent pieces: several features, tickets, pages, variants or files that could each be done and checked on their own; or a single task, question or reply"
        }),
    )
}

fn is_correction_question() -> Value {
    noul_question(
        "Is this new message a reaction to the previous answer being wrong, incomplete, or not what the user wanted?",
        json!({
            "true": "a correction, complaint, retry, or the same question asked again; the user is unhappy with or re-asking about what the assistant just did",
            "false": "a new task, a follow-up building on a good answer, a clarification the user was asked for, or an acknowledgement"
        }),
    )
}

/// Used only by the batch: the task-type judgment has no single-call twin.
fn continues_question() -> Value {
    noul_question(
        "Does this message continue the task already in progress (a follow-up on the same work), rather than start a new task?",
        json!({
            "true": "a follow-up on the work just discussed: a tweak, a fix to the last change, 'now do the same for X', 'why did that fail', a short reply that only makes sense given the previous turn",
            "false": "a new, self-contained task or question about something else, even if it is in the same repository"
        }),
    )
}

/// Which kind of task a coding request is. Each option says what it is FOR and what
/// it is NOT for, because neighbours overlap (security vs review, feature vs
/// simple_edit, explain vs docs) and Jev separates them best when told where each
/// boundary is. Names are `TaskType::as_str`, so the answer parses straight back.
fn task_type_question() -> Value {
    choice_question(
        "What kind of task is this coding request?",
        json!({
            "security": {
                "what": "authorization and access control, injection, secrets and credentials, crypto, handling of untrusted input, or reviewing/fixing code for vulnerabilities",
                "not_for": "ordinary auth plumbing like adding a login page, or security-sounding words that mean something else (design tokens, React keys)",
                "examples": ["can a member read another tenant's files", "check this route for path traversal", "how are API keys stored"]
            },
            "hard_debug": {
                "what": "bugs that span files or systems, concurrency, deadlocks, races, performance mysteries, root-causing flaky failures",
                "not_for": "an error with an obvious local fix, or a bug in one small function given in the message",
                "examples": ["why does the upload hang on large files", "this test fails one run in ten", "memory grows until the worker is killed"]
            },
            "review": {
                "what": "reviewing a diff, PR or file for bugs and risks and reporting findings",
                "not_for": "making the changes themselves (feature), or a review that is specifically about vulnerabilities (security)",
                "examples": ["review this PR", "look over my changes before I merge", "anything wrong with this module?"]
            },
            "feature": {
                "what": "building or extending functionality across one or several files, refactors, migrations, writing tests for new behaviour",
                "not_for": "a one-line or purely mechanical change (simple_edit), or UI-only work (frontend)",
                "examples": ["add pagination to the list endpoint", "migrate the store from Redux to Zustand", "implement retry with backoff"]
            },
            "frontend": {
                "what": "UI components, styling, layout, responsive design, accessibility",
                "not_for": "backend or data logic that merely feeds a UI",
                "examples": ["make the sidebar collapse on mobile", "fix the button focus ring", "build a settings page layout"]
            },
            "docs": {
                "what": "READMEs, code comments, changelogs, and explanations written for people to read and keep",
                "not_for": "answering a question about code that nobody will keep (explain)",
                "examples": ["write a README for this package", "document the public API", "add a changelog entry"]
            },
            "explain": {
                "what": "read-only questions about how existing code works or where something is",
                "not_for": "anything that changes files, or needs facts from outside the repository",
                "examples": ["how does the cache invalidation work", "where is the config loaded", "does jev-code has plan mode?", "does this repo support X", "is there a /foo command in jev-code"]
            },
            "current_info": {
                "what": "needs recent facts the model may not know: library versions, new APIs, release notes, web search",
                "not_for": "questions about the current repository, codebase, or jev-code and its commands or features, which are explain; or questions answerable from long-standing knowledge",
                "examples": ["what is the latest version of axum", "does the new React compiler change this", "is this API deprecated now"]
            },
            "simple_edit": {
                "what": "rename, tweak a config value, a one-liner, boilerplate, a small mechanical change",
                "not_for": "anything needing design, several coordinated changes, or debugging",
                "examples": ["rename usr to user", "bump the timeout to 30s", "add a .gitignore entry"]
            },
            "unclear": {
                "what": "ambiguous, mixed, or too short to tell what kind of task it is, such as a bare acknowledgement"
            }
        }),
    )
}

/// The questions of one provider judgment, keyed as `decode_provider_want` reads them.
const Q_TASK: &str = "task_type";
const Q_CONTINUES: &str = "continues";
const Q_EDIT: &str = "needs_changes";
const Q_SECURITY: &str = "security_review";
const Q_HARD: &str = "very_hard";
const Q_CORRECTION: &str = "is_correction";
const Q_MULTI_PART: &str = "multi_part";
const Q_SEQUENTIAL: &str = "sequential";

/// Every question of a provider judgment as one `questions` map. The correction
/// question needs the previous exchange, so it is asked only when there is one; the
/// `sequential` veto only matters to orchestration, so it is asked only when that is on.
fn provider_questions(with_previous: bool, with_sequence: bool) -> Value {
    let mut q = json!({
        Q_TASK: task_type_question(),
        Q_CONTINUES: continues_question(),
        Q_EDIT: needs_changes_question(),
        Q_SECURITY: security_review_question(),
        Q_HARD: very_hard_question(),
        Q_MULTI_PART: multi_part_question(),
    });
    if with_previous {
        q[Q_CORRECTION] = is_correction_question();
    }
    if with_sequence {
        q[Q_SEQUENTIAL] = sequential_question();
    }
    q
}

/// The shared state for the batch: the usual request state, plus the previous exchange
/// when there is one (the correction question reads it).
fn provider_state(request: &str, recent_context: &str, previous: Option<(&str, &str)>) -> Value {
    let mut state = request_state(request, recent_context);
    if let Some((prev_request, prev_answer)) = previous {
        state["previous_request"] = json!(prev_request);
        state["previous_answer"] = json!(prev_answer);
    }
    state
}

/// Turn the batch's `answers` into a [`ProviderWant`]. An unknown task type name is an
/// error rather than a guess: the caller falls back to its current model. The `sequential`
/// answer is the exception: it only feeds the orchestration veto, so when it cannot be read
/// the judgment keeps every other signal, has no veto information, and the decision log
/// says why (`sequential_error`).
fn decode_provider_want(answers: &Value, request: &str, with_previous: bool, with_sequence: bool) -> Result<ProviderWant> {
    let task_answer = decode_choice(answers, Q_TASK)?;
    let task: TaskType = task_answer.choice.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let task = repo_question_task(request, task);
    let very_hard_p = decode_noul(answers, Q_HARD)?;
    let security_p = decode_noul(answers, Q_SECURITY)?;
    let edit_p = decode_noul(answers, Q_EDIT)?;
    let continues_p = decode_noul(answers, Q_CONTINUES)?;
    let sequential = with_sequence.then(|| decode_noul(answers, Q_SEQUENTIAL));
    let sequential_error = sequential.as_ref().and_then(|r| r.as_ref().err()).map(|e| format!("{e:#}"));
    let orchestration = Orchestration::judged(decode_noul(answers, Q_MULTI_PART)?, sequential.and_then(Result::ok));
    let correction_p = if with_previous { Some(decode_noul(answers, Q_CORRECTION)?) } else { None };
    let security_keywords = security_level(request);
    let mut signals = json!({
        "task": task.as_str(), "task_confidence": task_answer.confidence,
        "very_hard_p": very_hard_p, "security_p": security_p, "edit_p": edit_p,
        "continues_p": continues_p, "multi_part_p": orchestration.multi_part_p,
        "sequential_p": orchestration.sequential_p, "orch_vetoed": orchestration.vetoed, "correction_p": correction_p,
        "security": format!("{security_keywords:?}").to_lowercase(),
    });
    if let Some(e) = sequential_error {
        signals["sequential_error"] = json!(e);
    }
    Ok(ProviderWant {
        task,
        task_confidence: task_answer.confidence,
        very_hard_p,
        security_p,
        edit_p,
        continues_p,
        correction_p,
        orchestration,
        security_keywords,
        signals,
    })
}

/// Correct a current-info judgment for explicit questions about this local tool or repository.
fn repo_question_task(request: &str, task: TaskType) -> TaskType {
    let lower = request.to_ascii_lowercase();
    let question = ["does ", "do ", "is ", "are ", "can ", "how ", "where ", "what "].iter().any(|prefix| lower.trim_start().starts_with(prefix));
    let local = ["jev-code", "this repo", "this codebase", "this repository"].iter().any(|s| lower.contains(s));
    let external = ["latest", "today", "news", "release notes"].iter().any(|s| lower.contains(s));
    if task == TaskType::CurrentInfo && question && local && !external { TaskType::Explain } else { task }
}

/// Strong signals: vulnerability classes, secrets, crypto, auditing. Matched as
/// whole words (lowercased, split on non-alphanumerics) so "author" or "HashMap"
/// don't hit.
const SECURITY_STRONG_WORDS: &[&str] = &[
    "idor", "injection", "xss", "csrf", "ssrf", "rce", "traversal", "pollution",
    "privilege", "privileges", "exploit", "exploits", "unsafe", "arbitrary",
    "secret", "secrets", "password", "passwords", "passwd", "passphrase",
    "credential", "credentials", "apikey", "hmac", "signature", "signatures",
    "bcrypt", "argon2", "scrypt", "pbkdf2", "cipher", "csp", "escape", "escaping",
    "secure", "securely", "security", "allowlist", "allowlists", "denylist", "blocklist",
    "isolation", "tenant", "tenants", "leak", "leaks", "leakage", "leaking", "redos", "untrusted",
    "bypass", "forge", "forged", "forgery", "noopener", "tabnabbing", "dangerously",
];
const SECURITY_STRONG_PREFIXES: &[&str] = &[
    "vulnerab", "sanitiz", "sanitis", "escalat", "audit", "crypt", "encrypt", "decrypt",
    "inject", "poison", "exfiltrat",
];
const SECURITY_STRONG_PHRASES: &[&str] =
    &["api key", "access key", "private key", "mass assignment", "path travers", "javascript:"];

/// Weak signals: auth plumbing. Routine to read about, risky to change.
const SECURITY_WEAK_WORDS: &[&str] = &[
    "auth", "authn", "authz", "oauth", "oauth2", "oidc", "saml", "sso", "jwt", "jwts",
    "login", "logout", "signin", "signup", "session", "sessions", "cookie", "cookies",
    "token", "tokens", "cors", "permission", "permissions", "rbac", "tls", "ssl",
    "certificate", "certificates", "webhook", "webhooks", "admin", "parameterize",
    "parameterized", "redirect", "redirects", "origin", "origins", "postmessage", "turnstile",
    "captcha",
];
const SECURITY_WEAK_PREFIXES: &[&str] = &["authenticat", "authoriz", "sandbox"];
/// Common non-security uses of weak words, removed before matching.
const SECURITY_WEAK_EXCLUDES: &[&str] = &["design token", "color token", "css token", "theme token"];

/// "securityLevel" → "security Level", so identifiers match like words. Keeps the
/// original word too ("postMessage" also stays "postmessage").
fn split_camel(text: &str) -> String {
    let mut split = String::with_capacity(text.len() + 8);
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_uppercase() && prev_lower {
            split.push(' ');
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        split.push(c);
    }
    format!("{text} {split}")
}

/// How security-sensitive a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Security {
    None,
    /// Touches auth/session/token plumbing: fine to explain, risky to change.
    Weak,
    /// Vulnerabilities, secrets, crypto, audits: always the strong model.
    Strong,
}

pub fn security_level(request: &str) -> Security {
    let lower = split_camel(request).to_lowercase();
    if SECURITY_STRONG_PHRASES.iter().any(|p| lower.contains(p)) {
        return Security::Strong;
    }
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let hit = |w: &str, list: &[&str], prefixes: &[&str]| {
        list.contains(&w) || prefixes.iter().any(|p| w.starts_with(p))
    };
    if words(&lower).iter().any(|w| hit(w, SECURITY_STRONG_WORDS, SECURITY_STRONG_PREFIXES)) {
        return Security::Strong;
    }
    let mut weak_text = lower.clone();
    for ex in SECURITY_WEAK_EXCLUDES {
        weak_text = weak_text.replace(ex, " ");
    }
    if words(&weak_text).iter().any(|w| hit(w, SECURITY_WEAK_WORDS, SECURITY_WEAK_PREFIXES)) {
        return Security::Weak;
    }
    Security::None
}

/// Chat mode's hard rule: any security signal goes to the strong model, because
/// a confident-looking wrong answer is costly here and Jev's difficulty judgment
/// alone doesn't reliably catch it.
pub fn is_security_sensitive(request: &str) -> bool {
    security_level(request) != Security::None
}

/// A short, context-dependent follow-up ("go on", "why?", "shorter please").
pub fn is_short_follow_up(request: &str) -> bool {
    request.split_whitespace().count() <= 8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(tier: usize, confident: bool) -> CodeWant {
        CodeWant { tier, confident, reason: String::new(), signals: Value::Null }
    }

    #[test]
    fn uncertain_turn_holds_downgrade_streak() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        assert_eq!(s.decide(&want(1, true), 2).0, 2); // 1/2
        assert_eq!(s.decide(&want(1, false), 2).0, 2); // vague turn: streak kept
        assert_eq!(s.decide(&want(1, true), 2).0, 1); // 2/2: step down
    }

    #[test]
    fn workers_follow_the_planner_haiku_only_for_read_only_lookups_when_enabled() {
        let lookup = json!({ "choice": "simple", "confidence": 0.95, "edit_p": 0.05 });
        let edit = json!({ "choice": "simple", "confidence": 0.95, "edit_p": 0.7 });
        assert_eq!(worker_alias(1, &lookup, None, false), "sonnet", "Haiku is off unless enabled");
        assert_eq!(worker_alias(1, &lookup, Some(0.9), false), "haiku");
        assert_eq!(worker_alias(1, &edit, Some(0.9), false), "sonnet", "anything that edits stays off Haiku");
        assert_eq!(worker_alias(1, &json!({ "choice": "simple", "confidence": 0.6, "edit_p": 0.05 }), Some(0.9), false), "sonnet");
        // Opus only under a Fable planner, and only for security or hard tasks
        assert_eq!(worker_alias(2, &lookup, Some(0.9), false), "sonnet", "Opus planner: no Opus workers");
        assert_eq!(worker_alias(2, &lookup, Some(0.9), true), "opus", "Fable planner: Opus for hard or security tasks");
        assert_eq!(worker_alias(1, &edit, None, true), "sonnet", "an everyday task is still Sonnet");
        assert_eq!(worker_alias(3, &edit, None, false), "sonnet");
    }

    #[test]
    fn top_tier_steps_down_one_tier_at_a_time() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        let (t, why) = s.decide(&want(3, true), 2);
        assert_eq!((t, why.starts_with("upgrade")), (3, true));
        s.tier = Some(3);
        assert_eq!(s.decide(&want(1, true), 2).0, 3);
        let (t, _) = s.decide(&want(1, true), 2);
        assert_eq!(t, 2, "Fable steps down to Opus, not straight to Sonnet");
        s.tier = Some(2);
        assert_eq!(s.decide(&want(1, true), 2).0, 2);
    }

    #[test]
    fn hard_turn_resets_downgrade_streak() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        assert_eq!(s.decide(&want(1, true), 2).0, 2);
        assert_eq!(s.decide(&want(2, true), 2).0, 2); // needs Opus: streak reset
        assert_eq!(s.decide(&want(1, true), 2).0, 2); // back to 1/2
    }

    #[test]
    fn vague_turns_cannot_hold_a_tier_forever() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        for _ in 1..MAX_HOLD {
            assert_eq!(s.decide(&want(1, false), 2).0, 2);
        }
        assert_eq!(s.decide(&want(1, false), 2).0, 1);
    }

    #[tokio::test]
    async fn a_stalled_jev_connection_keeps_the_current_model_instead_of_hanging() {
        // Accepts the connection, then never answers.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });
        let router = Router::new(TypeSafe::with_url("test-key", &url), 1.0)
            .with_jev_timeout(std::time::Duration::from_millis(200))
            .with_top_threshold(Some(0.9));
        let started = std::time::Instant::now();
        let w = router.code_want("why does the upload hang on large files", "", 2).await;
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
        assert_eq!(w.tier, 2, "keeps the fallback (current) tier");
        assert!(!w.confident);
        assert!(w.reason.starts_with("jev unavailable (timed out"), "{}", w.reason);
        assert!(w.signals.is_null());
    }

    #[test]
    fn security_detection() {
        for r in [
            "hash passwords with bcrypt",
            "write a webhook handler that verifies an HMAC signature",
            "sanitize HTML input",
            "escape SQL strings",
            "generate secure random tokens",
            "store the API key in config",
            "encrypt this field",
            "check routes/projects.ts for path traversal",
            "Check pages.ts for IDOR",
            "Audit cards.ts routes for mass assignment",
            "can a member escalate their own role",
            "call_tool.py lets a workflow call tools by name. Is there an allowlist?",
            "Review storage.py for cross-tenant file leakage",
            "Is there any way to inject a system message through the API?",
            "is the mermaid securityLevel set to strict",
            "do we block javascript: and data: URLs when pasting links",
            "can markdown images exfiltrate data via query strings",
            "never use dangerouslySetInnerHTML here",
        ] {
            assert_eq!(security_level(r), Security::Strong, "{r}");
        }
        for r in [
            "verify a JWT token",
            "set CORS headers",
            "implement session management",
            "where is the login page",
            "where are the Better Auth config and session helpers",
            "Are there CPU and memory limits on sandboxed execution?",
            "How does use_sandbox.py decide which backend to use?",
            "check each window.addEventListener('message') handler validates the origin",
            "validate the redirect path in routes-utils.ts",
        ] {
            assert_eq!(security_level(r), Security::Weak, "{r}");
        }
        for r in [
            "write a Python dataclass for a book with title, author, and year",
            "use a HashMap to count words",
            "rename the variable usr to user",
            "what does HTTP status 404 mean",
            "where are the --ed-* design tokens defined",
            "thanks!",
        ] {
            assert_eq!(security_level(r), Security::None, "{r}");
        }
    }

    // ---- provider_want, against a local fake Jev ----

    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The bodies of the requests a fake Jev received, in order.
    type Seen = Arc<Mutex<Vec<Value>>>;

    /// Read one HTTP request off `sock` and return its JSON body.
    async fn read_json_request(sock: &mut tokio::net::TcpStream) -> Option<Value> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= head_end + 4 + len {
                return serde_json::from_slice(&buf[head_end + 4..head_end + 4 + len]).ok();
            }
        }
    }

    /// A fake Jev: for each request, `reply` makes (status, body). Returns its URL and
    /// the request bodies seen so far.
    async fn fake_jev(reply: fn(&Value) -> (u16, String)) -> (String, Seen) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let log = log.clone();
                tokio::spawn(async move {
                    let Some(req) = read_json_request(&mut sock).await else { return };
                    let (status, body) = reply(&req);
                    log.lock().unwrap().push(req);
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        (url, seen)
    }

    /// Answers every question that was asked: `frontend` at 0.8 for the choice, and a
    /// distinct probability per noul so a decoding mix-up shows.
    fn answer_all(req: &Value) -> (u16, String) {
        let mut answers = serde_json::Map::new();
        for (name, q) in req["questions"].as_object().unwrap() {
            let a = if q["type"] == "choice" {
                json!({ "choice": "frontend", "confidence": 0.8, "probabilities": { "frontend": 0.8, "docs": 0.2 } })
            } else {
                let p = match name.as_str() {
                    "continues" => 0.11,
                    "needs_changes" => 0.22,
                    "security_review" => 0.33,
                    "very_hard" => 0.44,
                    "is_correction" => 0.55,
                    "multi_part" => 0.66,
                    "sequential" => 0.11,
                    _ => 0.99,
                };
                json!({ "noul": p })
            };
            answers.insert(name.clone(), a);
        }
        (200, json!({ "answers": answers }).to_string())
    }

    fn router_at(url: &str, timeout_ms: u64) -> Router {
        Router::new(TypeSafe::with_url("test-key", url), 1.0)
            .with_jev_timeout(std::time::Duration::from_millis(timeout_ms))
    }

    #[tokio::test]
    async fn provider_want_decodes_every_answer_from_one_request() {
        let (url, seen) = fake_jev(answer_all).await;
        let w = router_at(&url, 2000)
            .provider_want("make the sidebar collapse", "ctx", Some(("earlier ask", "earlier answer")), true)
            .await
            .expect("answered");
        assert_eq!(w.task, TaskType::Frontend);
        assert_eq!(w.task_confidence, 0.8);
        assert_eq!((w.continues_p, w.edit_p, w.security_p, w.very_hard_p), (0.11, 0.22, 0.33, 0.44));
        assert_eq!(w.correction_p, Some(0.55));
        assert_eq!(w.orchestration.multi_part_p, 0.66);
        assert_eq!(w.security_keywords, Security::None);
        assert_eq!(w.signals["task"], "frontend");
        assert_eq!(w.signals["security_p"], 0.33);
        // exactly one request carried all seven questions and the previous exchange
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "one HTTP request, not one per question");
        assert_eq!(seen[0]["questions"].as_object().unwrap().len(), 8);
        assert_eq!(seen[0]["state"]["previous_answer"], "earlier answer");
        assert_eq!(seen[0]["state"]["request"], "make the sidebar collapse");
    }

    #[tokio::test]
    async fn provider_want_without_a_previous_turn_omits_the_correction_question() {
        let (url, seen) = fake_jev(answer_all).await;
        let w = router_at(&url, 2000).provider_want("rename usr to user", "", None, true).await.expect("answered");
        assert_eq!(w.correction_p, None);
        assert!(w.signals["correction_p"].is_null());
        let seen = seen.lock().unwrap();
        let questions = seen[0]["questions"].as_object().unwrap();
        assert_eq!(questions.len(), 7);
        assert_eq!(questions["multi_part"], multi_part_question(), "the batch sends the shared definition");
        assert!(!questions.contains_key("is_correction"));
        assert!(seen[0]["state"].get("previous_request").is_none());
    }

    #[tokio::test]
    async fn with_orchestration_off_the_sequence_veto_is_not_asked() {
        let (url, seen) = fake_jev(answer_all).await;
        let w = router_at(&url, 2000).provider_want("rename usr to user", "", None, false).await.expect("answered");
        assert_eq!((w.orchestration.multi_part_p, w.orchestration.sequential_p, w.orchestration.vetoed), (0.66, None, false));
        assert!(w.signals["sequential_p"].is_null() && w.signals.get("sequential_error").is_none());
        let seen = seen.lock().unwrap();
        let questions = seen[0]["questions"].as_object().unwrap();
        assert_eq!(questions.len(), 6);
        assert!(!questions.contains_key("sequential"), "asked only when orchestration is on");
    }

    /// Every answer readable except `sequential`, which has no probability.
    fn unreadable_sequential(req: &Value) -> (u16, String) {
        let (_, body) = answer_all(req);
        let mut body: Value = serde_json::from_str(&body).unwrap();
        if body["answers"].get("sequential").is_some() {
            body["answers"]["sequential"] = json!({ "noul": "not a number" });
        }
        (200, body.to_string())
    }

    #[tokio::test]
    async fn an_unreadable_sequential_answer_loses_only_the_veto() {
        let (url, _) = fake_jev(unreadable_sequential).await;
        let router = router_at(&url, 2000);
        let w = router.provider_want("make the sidebar collapse", "", None, true).await.expect("the other signals still route");
        assert_eq!(w.task, TaskType::Frontend);
        assert_eq!((w.continues_p, w.edit_p, w.security_p, w.very_hard_p), (0.11, 0.22, 0.33, 0.44));
        assert_eq!((w.orchestration.multi_part_p, w.orchestration.sequential_p, w.orchestration.vetoed), (0.66, None, false));
        assert!(w.signals["sequential_error"].as_str().is_some_and(|e| !e.is_empty()), "the log says why: {}", w.signals);
        // The gateway's check degrades the same way.
        let j = router.orchestration_p("make the sidebar collapse", "").await.expect("multi_part was readable");
        assert_eq!((j.multi_part_p, j.sequential_p, j.vetoed), (0.66, None, false));
    }

    /// A multi-part request that is also a sequence: the veto holds it back.
    fn multi_part_but_sequential(req: &Value) -> (u16, String) {
        let mut answers = serde_json::Map::new();
        for name in req["questions"].as_object().unwrap().keys() {
            let p = match name.as_str() {
                "multi_part" => 0.95,
                "sequential" => 0.9,
                _ => 0.1,
            };
            answers.insert(name.clone(), json!({ "noul": p }));
        }
        (200, json!({ "answers": answers }).to_string())
    }

    #[tokio::test]
    async fn orchestration_p_keeps_the_raw_judgment_and_the_veto() {
        let (url, seen) = fake_jev(multi_part_but_sequential).await;
        let j = router_at(&url, 2000).orchestration_p("do 1, then 2, then 3", "").await.expect("answered");
        assert_eq!((j.multi_part_p, j.sequential_p, j.vetoed), (0.95, Some(0.9), true));
        let fields = Orchestration::fields(Some(&j));
        assert_eq!(fields["multi_part_p"], 0.95, "the log keeps the raw judgment");
        assert_eq!((fields["sequential_p"].clone(), fields["vetoed"].clone()), (json!(0.9), json!(true)));
        assert_eq!(seen.lock().unwrap().len(), 1, "one request carries both questions");
    }

    #[test]
    fn a_parallel_request_is_not_vetoed_and_no_answer_logs_nulls() {
        let j = Orchestration::new(0.9, 0.2);
        assert_eq!((j.multi_part_p, j.vetoed), (0.9, false));
        assert_eq!(Orchestration::fields(Some(&j))["vetoed"], false);
        let none = Orchestration::fields(None);
        for key in ["multi_part_p", "sequential_p", "vetoed"] {
            assert!(none[key].is_null(), "{key}");
        }
    }

    #[test]
    fn local_questions_are_not_current_info() {
        for prompt in ["does jev-code has plan mode?", "does this repo support X", "is there a /foo command in jev-code"] {
            assert_eq!(repo_question_task(prompt, TaskType::CurrentInfo), TaskType::Explain, "{prompt}");
        }
        for prompt in ["what's the latest rust version", "today's news about X", "what is the latest release of jev-code"] {
            assert_eq!(repo_question_task(prompt, TaskType::CurrentInfo), TaskType::CurrentInfo, "{prompt}");
        }
    }

    #[test]
    fn provider_want_task_options_are_exactly_the_ten_task_types() {
        let q = task_type_question();
        let mut options: Vec<&str> = q["criteria"].as_object().unwrap().keys().map(String::as_str).collect();
        let mut expected: Vec<&str> = TaskType::ALL.iter().map(|t| t.as_str()).collect();
        options.sort_unstable();
        expected.sort_unstable();
        assert_eq!(options, expected);
    }

    #[tokio::test]
    async fn a_stalled_jev_gives_no_provider_want_within_the_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });
        let started = std::time::Instant::now();
        let w = router_at(&url, 200).provider_want("why does the upload hang", "", None, true).await;
        assert!(w.is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_jev_error_gives_no_provider_want() {
        let (url, _) = fake_jev(|_| (500, "boom".into())).await;
        assert!(router_at(&url, 2000).provider_want("anything", "", None, true).await.is_none());
        // a well-formed reply with an unknown task name is no answer either
        let (url, _) = fake_jev(|_| {
            (200, json!({ "answers": { "task_type": { "choice": "astrology", "confidence": 1.0 } } }).to_string())
        })
        .await;
        assert!(router_at(&url, 2000).provider_want("anything", "", None, true).await.is_none());
    }

    #[tokio::test]
    async fn the_single_call_method_sends_the_shared_question_text() {
        let (url, seen) = fake_jev(|_| (200, json!({ "answers": { "q": { "noul": 0.7 } } }).to_string())).await;
        let p = router_at(&url, 2000).security_review("can a viewer see X?", "").await.unwrap();
        assert_eq!(p, 0.7);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0]["questions"]["q"], security_review_question());
        assert_eq!(seen[0]["state"], json!({ "request": "can a viewer see X?", "recent_context": "" }));
    }

    #[test]
    fn question_wire_format_is_unchanged() {
        let q = needs_changes_question();
        assert_eq!(q["type"], "noul");
        assert_eq!(
            q["instructions"],
            "Does fulfilling this request require modifying files or running commands in the project?"
        );
        assert!(q["criteria"]["true"].as_str().unwrap().starts_with("editing, creating"));
        assert_eq!(choice_question("i", json!({})), json!({ "type": "choice", "instructions": "i", "criteria": {} }));
    }

    #[test]
    fn a_sequence_is_vetoed_and_parallel_work_is_not() {
        assert!(!Orchestration::new(0.89, 0.10).vetoed, "independent pieces");
        assert!(!Orchestration::new(0.89, 0.69).vetoed, "below the veto");
        assert!(Orchestration::new(0.89, 0.90).vetoed, "do 1 -> 2 -> 3 is not orchestrated");
        assert_eq!(Orchestration::new(0.89, 0.90).multi_part_p, 0.89, "the raw answer is never rewritten");
    }
}
