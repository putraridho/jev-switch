//! The user's own model chains: reads the setting that names the file and hands it to the
//! catalog.

use crate::catalog::domain::model_table::Overrides;
use crate::shared::application::host;

/// [`Overrides::from_setting`] for the file named by `JEV_CHAINS`.
pub fn from_env() -> Overrides {
    Overrides::from_setting(host::env("JEV_CHAINS"))
}
