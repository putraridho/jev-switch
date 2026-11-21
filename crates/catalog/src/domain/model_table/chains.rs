use super::models::{allowed_for, by_id};
use super::{ModelSpec, Overrides, TaskType};

/// The default best-first model ids for a task. Entries flagged `very_hard_only` in
/// [`models`] are dropped by [`chain_with`] unless the turn is very hard, so the
/// Gemini Carbon entry in the Security list needs no special case here.
fn default_ids(task: TaskType) -> &'static [&'static str] {
    match task {
        TaskType::Security => &[
            "claude-fable-5-2",
            "gpt-6-astra",
            "claude-opus-5-5",
            "gpt-6.1-sol",
            "gemini-4-carbon",
            "gemini-4-neon",
            "gemini-4-argon",
        ],
        TaskType::HardDebug => &[
            "gpt-6-astra",
            "claude-fable-5-2",
            "gpt-6.1-sol",
            "claude-opus-5-5",
            "gemini-4-carbon",
            "gemini-4-neon",
            "gemini-4-argon",
            "deepseek-v4-pro",
        ],
        TaskType::Review => &[
            "gpt-6.1-sol",
            "gemini-4-neon",
            "gemini-4-argon",
            "claude-opus-5-5",
            "deepseek-v4-pro",
        ],
        TaskType::Feature | TaskType::Frontend | TaskType::Docs => {
            &["claude-sonnet-5-5", "gpt-6.1-terra", "gemini-4-hydrogen", "deepseek-v4-pro"]
        }
        TaskType::Explain => &["grok-4.7", "gemini-3.8-flash", "deepseek-flash", "claude-sonnet-5-5"],
        TaskType::CurrentInfo => &["grok-4.7", "gemini-3.8-flash", "claude-sonnet-5-5", "gpt-6.1-terra"],
        TaskType::SimpleEdit => &["deepseek-flash", "gpt-6-luna", "gemini-3.8-flash", "claude-sonnet-5-5"],
        TaskType::Unclear => &["claude-sonnet-5-5", "gpt-6.1-terra", "gemini-4-hydrogen"],
    }
}

/// Apply the two filters every chain must pass, whatever its source.
fn admit(
    specs: impl IntoIterator<Item = &'static ModelSpec>,
    task: TaskType,
    very_hard: bool,
) -> Vec<&'static ModelSpec> {
    specs.into_iter().filter(|s| (very_hard || !s.very_hard_only) && allowed_for(task, s)).collect()
}

/// Best-first models for `task`; `very_hard_only` models appear only if `very_hard`.
pub fn default_chain(task: TaskType, very_hard: bool) -> Vec<&'static ModelSpec> {
    admit(default_ids(task).iter().map(|id| by_id(id)), task, very_hard)
}

/// Like [`default_chain`], but an override replaces the default list. The hard
/// filters still apply to overrides, so a config file cannot send Security to DeepSeek.
pub fn chain_with(task: TaskType, very_hard: bool, overrides: &Overrides) -> Vec<&'static ModelSpec> {
    match overrides.get(task) {
        Some(list) => admit(list.iter().copied(), task, very_hard),
        None => default_chain(task, very_hard),
    }
}
