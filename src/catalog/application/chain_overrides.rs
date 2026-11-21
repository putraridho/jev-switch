//! The user's own model chains: reads the setting that names the file and hands it to the
//! catalog.

use crate::catalog::domain::model_table::Overrides;
use crate::shared::application::host::{self, Host};

/// [`Overrides::from_setting`] for the file named by `JEV_CHAINS`.
pub fn from_host(host: &dyn Host) -> Overrides {
    Overrides::from_setting(host.env("JEV_CHAINS"))
}

/// TEMPORARY: [`from_host`] through the process-wide host lookup, kept only for
/// `providers::application::provider_registry::probing` until it is handed a host.
pub fn from_env() -> Overrides {
    Overrides::from_setting(host::env("JEV_CHAINS"))
}
