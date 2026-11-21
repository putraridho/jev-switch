//! The real delegator, built on first use: probing providers takes seconds and the MCP
//! handshake must not wait for it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::OnceCell;

use crate::judgment::application::judge::Router;
use crate::judgment::infrastructure::typesafe_http::TypeSafe;
use crate::orchestration::application::delegator::Delegator;
use crate::orchestration::application::ports::{router_judge, BoxFut, DelegateApi, WorkerPolicy};
use crate::orchestration::domain::delegation::DelegateRequest;
use crate::orchestration::infrastructure::adapters;
use crate::orchestration::infrastructure::git_worktrees::GitWorktrees;
use crate::orchestration::infrastructure::worker_file::Store;
use crate::providers::application::provider_registry::Runtime;
use crate::shared::infrastructure::jsonl_log as log;

/// The privacy policy and permission mode workers run under, built from the state directory
/// and the project: the binary supplies it (it lives outside this context).
pub type PolicyFor = Box<dyn Fn(&Path, &Path) -> Arc<dyn WorkerPolicy> + Send + Sync>;

/// Builds the real [`Delegator`] on first use: probing providers takes seconds and the
/// MCP handshake must not wait for it.
pub(super) struct Lazy {
    pub(super) dir: PathBuf,
    pub(super) planner: String,
    pub(super) cell: OnceCell<Arc<Delegator>>,
    pub(super) policy_for: PolicyFor,
}

impl Lazy {
    async fn build(&self) -> Result<Arc<Delegator>> {
        let router = Arc::new(Router::new(TypeSafe::from_env()?, 1.0));
        let state = log::state_dir().context("no state directory (is HOME set?)")?;
        let worktrees = GitWorktrees::new(&self.dir, state.join("worktrees"))?;
        let runtime = Runtime::start(std::sync::Arc::new(crate::providers::infrastructure::probe_facts::SystemProbes)).await;
        let delegator = Delegator::new(runtime, router_judge(router), Box::new(worktrees), adapters::real((self.policy_for)(&state, &self.dir)))
            .with_log(log::path_from_env("JEV_DELEGATE_LOG", "orchestration.jsonl"))
            .with_planner(self.planner.clone())
            .with_auto_merge(auto_merge_on(crate::shared::infrastructure::env_file::lookup("JEV_AUTO_MERGE").as_deref()))
            .with_records(Arc::new(Store { dir: state.join("workers") }));
        // Workers an earlier run left: reload the mergeable ones, remove the abandoned ones.
        delegator.recover().await;
        Ok(Arc::new(delegator))
    }

    pub(super) async fn delegator(&self) -> Result<&Arc<Delegator>> {
        self.cell.get_or_try_init(|| self.build()).await
    }

    /// The delegator, built on demand (after a restart a worker lives only in the saved
    /// registry), or why delegation is unavailable.
    async fn known(&self) -> Result<&Arc<Delegator>, String> {
        self.delegator().await.map_err(|e| format!("delegation is not available: {e:#}"))
    }
}

/// Every call goes to the real delegator's own [`DelegateApi`] once it is built.
impl DelegateApi for Lazy {
    fn start(&self, req: DelegateRequest) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move {
            match self.known().await {
                Ok(d) => DelegateApi::start(d, req).await,
                Err(e) => Err(format!("{e}. Do this piece yourself.")),
            }
        })
    }

    fn wait(&self, task_ids: Option<Vec<String>>, timeout: Option<Duration>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { DelegateApi::wait(self.known().await?, task_ids, timeout).await })
    }

    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { DelegateApi::show_diff(self.known().await?, worker_id, file, offset).await })
    }

    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { DelegateApi::merge(self.known().await?, worker_id, files).await })
    }

    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { DelegateApi::discard(self.known().await?, worker_id).await })
    }

    fn undo(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { DelegateApi::undo(self.known().await?, worker_id).await })
    }
}

/// Auto-merge is on unless `JEV_AUTO_MERGE` says off (`0`, `off`, `false`, `no`).
pub(super) fn auto_merge_on(setting: Option<&str>) -> bool {
    !setting.map(str::trim).is_some_and(|v| ["0", "off", "false", "no"].iter().any(|off| v.eq_ignore_ascii_case(off)))
}
