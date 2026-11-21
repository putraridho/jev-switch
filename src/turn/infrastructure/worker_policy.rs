//! The [`WorkerPolicy`] of a delegated worker: the same privacy policy, secret scan and audit
//! log the planner's own turns answer to ([`Guards`]), and the permission mode and project
//! trust a worker's turn runs under.

use std::path::Path;

use crate::approval::application::extension_hooks::trust::project_settings_trusted;
use crate::approval::domain::mode::Mode;
use crate::backend::{ProviderId, TurnRequest};
use crate::orchestration::application::ports::{Screen, WorkerPolicy};
use crate::turn::application::guards::Guards;
use crate::turn::domain::privacy::outgoing;

/// Wraps the user's [`Guards`] (from `Guards::load`; `Guards::default()` for the default
/// secret scan only).
pub struct GuardedWorkers(pub Guards);

impl WorkerPolicy for GuardedWorkers {
    fn screen(&self, repo: &Path, outgoing_text: &str, brief: &str) -> Screen {
        let verdict = self.0.privacy.check(repo, outgoing_text, &[], self.0.home.as_deref());
        let permitted = crate::shared::ALL_PROVIDERS.into_iter().filter(|p| verdict.permits(*p)).collect();
        let judge_prompt = self.0.may_leave(repo, outgoing_text, &[]).then(|| outgoing::prepare(&self.0.privacy, brief).prompt);
        Screen { permitted, refusal: verdict.refusal_line(), judge_prompt }
    }

    fn outgoing(&self, prompt: &str, provider: ProviderId, model: &str, now_ms: u64) -> String {
        self.0.outgoing(prompt, provider, model, now_ms).prompt
    }

    fn request(&self, dir: &Path, model: &str, prompt: String, read_only: bool) -> TurnRequest {
        let mode = if read_only { Mode::Plan } else { Mode::default() };
        TurnRequest { dir: dir.to_path_buf(), model: model.to_string(), prompt, images: vec![], session_id: None, mode, trust_project_settings: project_settings_trusted(dir) }
    }
}
