//! Per-session routing and tracking state, bounded to the most recent sessions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::application::turns::Turns;
use crate::domain::prefix_fingerprint::PrefixWatch;
use jev_routing::domain::sticky::Sticky;
use jev_shared::application::host::Host;

const MAX_SESSIONS: usize = 1024;

pub struct SessionState {
    touched: Instant,
    pub tracking: Arc<SessionTracking>,
    pub sticky: Sticky,
    pub prompt_id: Option<String>,
    pub model: Option<&'static str>,
    /// Routing record of the current turn, attached to the usage rows of its requests.
    pub decision: Option<Value>,
    /// The latest orchestrate check for this session (the raw `multi_part_p`,
    /// `sequential_p`, `vetoed`), recorded on the next turn's decision whether or not it
    /// triggered, as a label for tuning.
    pub orch_check: Option<Value>,
    /// The planner model of the current turn when it is orchestrated, else `None`: a
    /// worker may be Opus only when this is Fable.
    pub orch_planner: Option<&'static str>,
}

pub struct SessionTracking {
    pub turns: Arc<Turns>,
    pub prefix: PrefixWatch,
}

/// The sessions by id, and the host each one's turn log writes through.
pub struct Sessions(HashMap<String, SessionState>, Arc<dyn Host>);

impl Sessions {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self(HashMap::new(), host)
    }

    pub fn entry(&mut self, id: &str, turns_path: &Option<String>) -> &mut SessionState {
        if !self.0.contains_key(id) && self.0.len() >= MAX_SESSIONS {
            if let Some(oldest) = self.0.iter().min_by_key(|(_, state)| state.touched).map(|(key, _)| key.clone()) {
                self.0.remove(&oldest);
            }
        }
        let state = self.0.entry(id.to_string()).or_insert_with(|| SessionState {
            touched: Instant::now(),
            tracking: Arc::new(SessionTracking { turns: Turns::new(turns_path.clone(), self.1.clone()), prefix: PrefixWatch::new() }),
            sticky: Sticky::default(),
            prompt_id: None,
            model: None,
            decision: None,
            orch_check: None,
            orch_planner: None,
        });
        state.touched = Instant::now();
        state
    }

    pub fn get(&mut self, id: &str) -> Option<&SessionState> {
        let state = self.0.get_mut(id)?;
        state.touched = Instant::now();
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_evict_the_oldest_tracking_with_their_routing_state() {
        let mut sessions = Sessions::new(Arc::new(jev_shared::infrastructure::system_host::SystemHost));
        let evicted = Arc::downgrade(&sessions.entry("oldest", &None).tracking);
        sessions.entry("oldest", &None).touched = Instant::now() - std::time::Duration::from_secs(10);
        for index in 0..MAX_SESSIONS - 1 {
            sessions.entry(&format!("session-{index}"), &None);
        }
        sessions.entry("new", &None);
        assert_eq!(sessions.0.len(), MAX_SESSIONS);
        assert!(sessions.get("oldest").is_none());
        assert!(evicted.upgrade().is_none());
        assert!(sessions.get("new").is_some());
    }
}
