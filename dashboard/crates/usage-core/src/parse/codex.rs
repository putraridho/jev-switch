//! Allow-list for Codex rate limits.
use crate::label::Label;
use crate::meter::{BalanceReading, LimitStatus, Meter, Meters, WindowKind, WindowReading};
use crate::quantity::{Currency, Money, Percent};
use crate::time::UnixSeconds;
use crate::usage::Unavailable;
use serde::de::IgnoredAny;
use serde::Deserialize;

/// What one rate-limit response says.
#[derive(Clone, PartialEq, Debug)]
pub struct CodexLimits {
    /// The windows and credit balance.
    pub meters: Meters,
    /// The plan name.
    pub plan: Option<Label>,
}
#[derive(Deserialize)]
struct Envelope<'a> {
    id: Option<u64>,
    #[serde(borrow)]
    result: Option<ResultBody<'a>>,
    error: Option<IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultBody<'a> {
    ordinary_usage_allowed: Option<bool>,
    #[serde(borrow)]
    rate_limits: Limits<'a>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limits<'a> {
    primary: Option<WindowData>,
    secondary: Option<WindowData>,
    #[serde(borrow)]
    credits: Option<CreditData<'a>>,
    plan_type: Option<&'a str>,
    rate_limit_reached_type: Option<IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowData {
    used_percent: f64,
    window_duration_mins: Option<u64>,
    resets_at: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreditData<'a> {
    has_credits: bool,
    unlimited: bool,
    balance: Option<&'a str>,
}
#[derive(Deserialize)]
struct Id {
    id: Option<u64>,
}
/// Whether the line has the requested response id.
pub fn is_response(line: &[u8], id: u64) -> bool {
    serde_json::from_slice::<Id>(line).is_ok_and(|v| v.id == Some(id))
}
/// Parse the requested rate limits response.
pub fn parse_rate_limits_response(
    line: &[u8],
    id: u64,
) -> Option<Result<CodexLimits, Unavailable>> {
    if !is_response(line, id) {
        return None;
    }
    let Ok(response) = serde_json::from_slice::<Envelope<'_>>(line) else {
        return Some(Err(Unavailable::Protocol));
    };
    if response.id != Some(id) {
        return None;
    }
    if response.error.is_some() {
        return Some(Err(Unavailable::ServerError));
    }
    let Some(result) = response.result else {
        return Some(Err(Unavailable::Protocol));
    };
    let status = if result.ordinary_usage_allowed == Some(false)
        || result.rate_limits.rate_limit_reached_type.is_some()
    {
        LimitStatus::Rejected
    } else {
        LimitStatus::Allowed
    };
    let mut meters = Vec::new();
    for (window, name) in [
        (result.rate_limits.primary, "primary"),
        (result.rate_limits.secondary, "secondary"),
    ] {
        let Some(window) = window else {
            continue;
        };
        let Some(used) = Percent::from_fraction(window.used_percent / 100.0) else {
            continue;
        };
        let kind = match window.window_duration_mins {
            Some(minutes) => WindowKind::from_minutes(minutes),
            None => match Label::parse(name) {
                Some(label) => WindowKind::Named { label },
                None => continue,
            },
        };
        let meter = Meter::Window(WindowReading {
            kind,
            used,
            resets_at: window.resets_at.map(UnixSeconds),
            status,
        });
        if !meters.iter().any(|m: &Meter| m.id() == meter.id()) {
            meters.push(meter);
        }
    }
    if let Some(credits) = result.rate_limits.credits {
        if credits.has_credits && !credits.unlimited {
            if let Some(total) = credits.balance.and_then(Money::parse_decimal) {
                meters.push(Meter::Balance(BalanceReading {
                    currency: Currency::Other,
                    total,
                    granted: None,
                    topped_up: None,
                    usable: true,
                }));
            }
        }
    }
    Some(
        Meters::new(meters)
            .map(|meters| CodexLimits {
                meters,
                plan: result.rate_limits.plan_type.and_then(Label::parse),
            })
            .ok_or(Unavailable::Protocol),
    )
}
