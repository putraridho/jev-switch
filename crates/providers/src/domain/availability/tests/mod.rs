//! Tests for the availability probes, by subject; the shared builder lives here.

mod decisions;
mod merging;
mod mise;
mod view;

use jev_backend::ProviderId;
use crate::domain::availability::{avail, Availability, State};

pub(super) fn ready(p: ProviderId, models: Option<Vec<&str>>) -> Availability {
    Availability {
        models: models.map(|m| m.into_iter().map(String::from).collect()),
        ..avail(p, State::Ready, "ok")
    }
}
