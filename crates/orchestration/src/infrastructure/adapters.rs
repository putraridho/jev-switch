//! The real adapters a delegator is built with: process liveness, the project's own test
//! command, the provider backends, the JSON-lines attempt log and the on-disk recovery scan.
//! `policy` (privacy, permission mode) comes from the composition root.

use std::sync::Arc;

use jev_backend::idle_timeout_from_env;
use crate::application::ports::{Adapters, WorkerPolicy};
use jev_providers::infrastructure::backend_factory::backend_for;
use jev_shared::application::host::Host;
use jev_shared::infrastructure::system_host::SystemHost;

use super::delegation_log::JsonlLog;
use super::process_liveness::ProcLiveness;
use super::worker_check::ProjectChecks;
use super::worker_file::gather_scan;

pub fn real(policy: Arc<dyn WorkerPolicy>) -> Adapters {
    Adapters {
        liveness: Arc::new(ProcLiveness),
        checks: Arc::new(ProjectChecks::default()),
        backend_factory: Box::new(backend_for),
        idle_timeout: idle_timeout_from_env(),
        log: Arc::new(JsonlLog),
        scan: Arc::new(gather_scan),
        policy,
        now: Box::new(|| SystemHost.now_ms()),
    }
}
