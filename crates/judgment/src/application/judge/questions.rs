//! The questions Jev is asked. One definition per yes/no question, used by the single-call
//! methods AND the batch in `provider_want`, so the two can never drift apart (a change
//! here changes both).

use serde_json::{json, Value};

use crate::application::wire::{choice_question, noul_question};

/// The state every code-mode question reads.
pub(super) fn request_state(request: &str, recent_context: &str) -> Value {
    json!({ "request": request, "recent_context": recent_context })
}

pub(super) fn needs_changes_question() -> Value {
    noul_question(
        "Does fulfilling this request require modifying files or running commands in the project?",
        json!({
            "true": "editing, creating, or deleting files; running builds, tests, scripts, git, or other commands",
            "false": "only reading or searching code and answering: explaining, locating, reviewing without changes"
        }),
    )
}

pub(super) fn security_review_question() -> Value {
    noul_question(
        "Is this request about security: who can access or change what, or whether something is safe against misuse or attack?",
        json!({
            "true": "authorization and access control (can a user, role, or member see or change something they shouldn't), data exposure, trust boundaries, untrusted input (injection, path traversal, SSRF, XSS, unsafe URLs or IPC), secrets and credentials, crypto, signatures, or reviewing code for vulnerabilities",
            "false": "ordinary coding, debugging, or design; or security-sounding words that mean something else here: design or CSS tokens, React keys, editor or canvas session state, hashing for ids, caching or dedupe, sandboxed iframe layout"
        }),
    )
}

pub(super) fn very_hard_question() -> Value {
    noul_question(
        "Is this one of the hardest requests, where only the most capable model will do and a mistake would be costly?",
        json!({
            "true": "designing or untangling a subtle architecture that spans many systems, debugging a deep concurrency, consistency or distributed-state problem, a security-critical design where a flaw would be costly, or reasoning that must stay exact across a very large amount of code",
            "false": "a question or explanation, a lookup, a routine feature, bug fix or refactor, reviewing a change, or anything a strong model would handle comfortably"
        }),
    )
}

pub(super) fn multi_part_question() -> Value {
    noul_question(
        "Is this request a large piece of work that splits into several separate steps, each changing a different part of the codebase?",
        json!({
            "true": "building or changing a feature across several files, layers or systems (API, UI, database, tests, docs), a migration or refactor touching many modules, or a list of distinct changes to make; each piece could be done and checked on its own",
            "false": "a question or explanation, a lookup, reviewing or investigating without a set of changes, a single bug fix or small edit in one place, a one-line command, or a short reply or follow-up to the conversation"
        }),
    )
}

pub(super) fn sequential_question() -> Value {
    noul_question(
        "Is this request a sequence of steps that must be done in order, each depending on the result of the previous one, rather than independent pieces that could be worked on at the same time?",
        json!({
            "true": "steps in a fixed order where a later step needs an earlier one's result: run the tests first, then check the report, then refactor; address one PR's comments, then the next stacked PR; or a short go-ahead for steps already discussed, such as \"do 1 -> 2 -> 3\" or \"go with the plan in that order\"",
            "false": "independent pieces: several features, tickets, pages, variants or files that could each be done and checked on their own; or a single task, question or reply"
        }),
    )
}

pub(super) fn is_correction_question() -> Value {
    noul_question(
        "Is this new message a reaction to the previous answer being wrong, incomplete, or not what the user wanted?",
        json!({
            "true": "a correction, complaint, retry, or the same question asked again; the user is unhappy with or re-asking about what the assistant just did",
            "false": "a new task, a follow-up building on a good answer, a clarification the user was asked for, or an acknowledgement"
        }),
    )
}

/// Used only by the batch: the task-type judgment has no single-call twin.
pub(super) fn continues_question() -> Value {
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
pub(super) fn task_type_question() -> Value {
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
pub(super) const Q_TASK: &str = "task_type";
pub(super) const Q_CONTINUES: &str = "continues";
pub(super) const Q_EDIT: &str = "needs_changes";
pub(super) const Q_SECURITY: &str = "security_review";
pub(super) const Q_HARD: &str = "very_hard";
pub(super) const Q_CORRECTION: &str = "is_correction";
pub(super) const Q_MULTI_PART: &str = "multi_part";
pub(super) const Q_SEQUENTIAL: &str = "sequential";
