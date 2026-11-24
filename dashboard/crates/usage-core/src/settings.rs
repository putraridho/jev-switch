//! The user's settings (DESIGN 7.4, 9.6). The types are frozen; T02 writes `validated` and
//! `apply`.

use crate::ipc::{CommandError, HourCycle, Theme};
use crate::provider::ProviderKey;
use crate::quantity::Percent;
use serde::{Deserialize, Serialize};

/// The version of the settings shape. A stored file with another version yields defaults.
pub const SETTINGS_VERSION: u32 = 1;

/// The warn and critical levels. Invariant: `warn < critical`; the fields are private so a
/// pair that breaks it cannot exist.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Thresholds {
    warn: Percent,
    critical: Percent,
}

impl Thresholds {
    /// `None` unless `warn < critical`.
    pub fn new(warn: Percent, critical: Percent) -> Option<Self> {
        (warn < critical).then_some(Self { warn, critical })
    }

    /// The warn level.
    pub fn warn(self) -> Percent {
        self.warn
    }

    /// The critical level.
    pub fn critical(self) -> Percent {
        self.critical
    }
}

/// Everything the user can change. Stored as JSON by `io::settings_file`; sent whole over IPC.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct Settings {
    /// The shape's version ([`SETTINGS_VERSION`]).
    pub version: u32,
    /// The warn level in basis points.
    pub warn_bp: u16,
    /// The critical level in basis points.
    pub critical_bp: u16,
    /// The master switch for notifications (also the tray checkbox).
    pub notifications: bool,
    /// Notify at the warn level.
    pub notify_warn: bool,
    /// Notify at the critical level.
    pub notify_critical: bool,
    /// Light, dark or the system's choice.
    pub theme: Theme,
    /// 12-hour, 24-hour or the locale's choice.
    pub hour_cycle: HourCycle,
    /// The zoom factor in percent: 100, 125, 150, 175 or 200.
    pub zoom_pct: u16,
    /// Closing the window keeps the app in the tray.
    pub close_to_tray: bool,
    /// Start hidden at login.
    pub autostart: bool,
    /// Single-key shortcuts are on.
    pub shortcuts: bool,
    /// The Claude live check (it spends a small request) is allowed.
    pub claude_live_check: bool,
    /// The first-run note has been shown.
    pub first_run_seen: bool,
    /// Providers the user has hidden.
    pub hidden_providers: Vec<ProviderKey>,
}

/// A change to some settings: an absent key means "unchanged". There is no `version`.
#[derive(Clone, Eq, PartialEq, Debug, Default, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct SettingsPatch {
    /// See [`Settings::warn_bp`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub warn_bp: Option<u16>,
    /// See [`Settings::critical_bp`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub critical_bp: Option<u16>,
    /// See [`Settings::notifications`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub notifications: Option<bool>,
    /// See [`Settings::notify_warn`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub notify_warn: Option<bool>,
    /// See [`Settings::notify_critical`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub notify_critical: Option<bool>,
    /// See [`Settings::theme`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub theme: Option<Theme>,
    /// See [`Settings::hour_cycle`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub hour_cycle: Option<HourCycle>,
    /// See [`Settings::zoom_pct`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub zoom_pct: Option<u16>,
    /// See [`Settings::close_to_tray`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub close_to_tray: Option<bool>,
    /// See [`Settings::autostart`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub autostart: Option<bool>,
    /// See [`Settings::shortcuts`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub shortcuts: Option<bool>,
    /// See [`Settings::claude_live_check`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub claude_live_check: Option<bool>,
    /// See [`Settings::first_run_seen`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub first_run_seen: Option<bool>,
    /// See [`Settings::hidden_providers`].
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(optional))]
    pub hidden_providers: Option<Vec<ProviderKey>>,
}

impl Settings {
    /// The settings of a fresh install (DESIGN 1a: autostart on, live check off).
    pub fn defaults() -> Self {
        Self {
            version: SETTINGS_VERSION,
            warn_bp: 8_000,
            critical_bp: 9_500,
            notifications: true,
            notify_warn: true,
            notify_critical: true,
            theme: Theme::System,
            hour_cycle: HourCycle::Auto,
            zoom_pct: 100,
            close_to_tray: true,
            autostart: true,
            shortcuts: true,
            claude_live_check: false,
            first_run_seen: false,
            hidden_providers: Vec::new(),
        }
    }

    /// The same settings with every value brought into range (thresholds 50..=100 % with
    /// warn below critical, zoom snapped to a step, a wrong version replaced by defaults).
    pub fn validated(self) -> Self {
        self
    }

    /// These settings with `p` applied; `InvalidArgument` when the result is out of range.
    pub fn apply(&self, p: SettingsPatch) -> Result<Self, CommandError> {
        let _ = p;
        Err(CommandError::Internal)
    }

    /// The warn and critical levels as a checked pair.
    pub fn thresholds(&self) -> Thresholds {
        let fallback = Thresholds {
            warn: Percent::from_basis_points(8_000),
            critical: Percent::from_basis_points(9_500),
        };
        Thresholds::new(
            Percent::from_basis_points(u32::from(self.warn_bp)),
            Percent::from_basis_points(u32::from(self.critical_bp)),
        )
        .unwrap_or(fallback)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_need_warn_below_critical() {
        let (w, c) = (
            Percent::from_basis_points(8_000),
            Percent::from_basis_points(9_500),
        );
        let t = Thresholds::new(w, c).unwrap();
        assert_eq!((t.warn(), t.critical()), (w, c));
        assert_eq!(Thresholds::new(c, w), None);
        assert_eq!(Thresholds::new(w, w), None);
    }

    #[test]
    fn settings_json_has_exactly_the_documented_keys() {
        let json = serde_json::to_string(&Settings::defaults()).unwrap();
        assert_eq!(
            json,
            concat!(
                r#"{"version":1,"warn_bp":8000,"critical_bp":9500,"notifications":true,"notify_warn":true,"#,
                r#""notify_critical":true,"theme":"system","hour_cycle":"auto","zoom_pct":100,"close_to_tray":true,"#,
                r#""autostart":true,"shortcuts":true,"claude_live_check":false,"first_run_seen":false,"#,
                r#""hidden_providers":[]}"#
            )
        );
        assert_eq!(
            serde_json::from_str::<Settings>(&json).unwrap(),
            Settings::defaults()
        );
    }

    #[test]
    fn patch_treats_an_absent_key_as_unchanged() {
        assert_eq!(
            serde_json::from_str::<SettingsPatch>("{}").unwrap(),
            SettingsPatch::default()
        );
        let patch: SettingsPatch = serde_json::from_str(
            r#"{"warn_bp":7000,"theme":"dark","hidden_providers":["deepseek"]}"#,
        )
        .unwrap();
        assert_eq!(patch.warn_bp, Some(7_000));
        assert_eq!(patch.theme, Some(Theme::Dark));
        assert_eq!(patch.hidden_providers, Some(vec![ProviderKey::DeepSeek]));
        assert_eq!(patch.critical_bp, None);
        assert_eq!(patch.autostart, None);
    }

    #[test]
    fn patch_refuses_values_outside_the_closed_vocabulary() {
        for bad in [
            r#"{"theme":"neon"}"#,
            r#"{"hidden_providers":["openai"]}"#,
            r#"{"warn_bp":-1}"#,
            r#"{"warn_bp":70000}"#,
            r#"{"autostart":"yes"}"#,
        ] {
            assert!(serde_json::from_str::<SettingsPatch>(bad).is_err(), "{bad}");
        }
    }
}
