//! Combining policies and the local-only switch.

use super::Policy;

impl Policy {
    /// `other`'s rules added to this one's. Local-only wins if either says so; redaction is
    /// off only when one of them says off (a project file cannot, see [`Source`]).
    pub fn merged(mut self, other: Policy) -> Self {
        self.local_only |= other.local_only;
        self.local_only_locked |= other.local_only_locked;
        self.redact &= other.redact;
        self.rules.extend(other.rules);
        self
    }

    /// The same policy with local-only forced on (`JEV_LOCAL_ONLY=1`, `/privacy local-only on`).
    pub fn with_local_only(mut self, on: bool) -> Self {
        self.local_only |= on;
        self.local_only_locked |= on;
        self
    }

    /// Turn local-only mode on or off for the session (unlike [`Self::with_local_only`],
    /// which can only add it). Turning it off is refused when a policy file or
    /// `JEV_LOCAL_ONLY` set it: those are the person's standing rule, not a session choice.
    pub fn set_local_only(&mut self, on: bool) -> Result<(), String> {
        if !on && self.local_only_locked {
            return Err("local-only is set by JEV_LOCAL_ONLY or a privacy.rules file; remove it there to turn it off".into());
        }
        self.local_only = on;
        Ok(())
    }

    pub fn local_only(&self) -> bool {
        self.local_only
    }

    pub fn redacts(&self) -> bool {
        self.redact
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}
