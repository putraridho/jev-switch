//! Collects the [`Inputs`] snapshot for the doctor from the running session and the machine.

use std::path::Path;

use crate::doctor::domain::diagnose::Inputs;
use super::machine_facts::{env_file_facts, in_git_repo, jev_status, state_dir_writable};
use crate::providers::application::provider_registry::{BlockReason, Runtime};
use crate::providers::infrastructure::cooldown_file;
use crate::shared::domain::duration_text::format_left;
use crate::shared::ProviderId;
use crate::shared::ALL_PROVIDERS;

/// Collect the facts. Thin IO: every probe is bounded and a failure becomes a fact.
pub async fn gather(runtime: &Runtime, dir: &Path) -> Inputs {
    let (jev_key_present, jev_reachable) = jev_status().await;
    Inputs {
        avail: runtime.avail.clone(),
        jev_key_present,
        jev_reachable,
        in_git_repo: in_git_repo(dir).await,
        state_dir_writable: Some(state_dir_writable()),
        env_file: env_file_facts(),
        cooldown_lines: cooldown_lines(runtime),
        blocked: blocked(runtime),
        allow_credits: runtime.allow.credits,
    }
}

/// In-memory cooldowns plus whatever other processes saved.
fn cooldown_lines(runtime: &Runtime) -> Vec<String> {
    let now = now_ms();
    let saved = runtime.cooldown_path.as_deref().map(cooldown_file::load).unwrap_or_default();
    let all = runtime.cooldowns.union(&saved);
    ALL_PROVIDERS.iter().filter_map(|p| Some(format!("{p} cooling for {}", format_left(all.active(*p, now)?)))).collect()
}

fn blocked(runtime: &Runtime) -> Vec<(ProviderId, String)> {
    ALL_PROVIDERS
        .iter()
        .filter_map(|p| Some((*p, block_reason_text(*p, *runtime.blocked.get(p)?))))
        .collect()
}

fn block_reason_text(provider: ProviderId, reason: BlockReason) -> String {
    match reason {
        BlockReason::Auth => "its login failed; log in again, then /providers refresh".to_string(),
        BlockReason::UnusableHere => match provider {
            ProviderId::Codex => "needs a git repository here; /new or another folder clears it".to_string(),
            ProviderId::Gemini => "needs a trusted workspace here; /new or another folder clears it".to_string(),
            _ => "refuses to run in this directory; /new or another folder clears it".to_string(),
        },
        BlockReason::Unsupported => match provider {
            ProviderId::Gemini => "Google retired the Gemini CLI login; install Antigravity (agy) and log in there".to_string(),
            _ => "its service no longer accepts this CLI; update or replace it".to_string(),
        },
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}


#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{Claude, Codex, Gemini};

    #[test]
    fn block_reasons_are_worded_per_provider() {
        assert!(block_reason_text(Codex, BlockReason::UnusableHere).contains("git repository"));
        assert!(block_reason_text(Gemini, BlockReason::UnusableHere).contains("trusted workspace"));
        assert!(block_reason_text(Claude, BlockReason::Auth).contains("/providers refresh"));
    }
}
