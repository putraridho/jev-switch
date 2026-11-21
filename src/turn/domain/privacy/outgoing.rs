//! What leaves the machine: the prompt with secrets blanked.

use super::Policy;
use crate::shared::domain::secrets;
use crate::shared::domain::secrets::SecretKind;

/// A prompt ready to send, and what was done to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub prompt: String,
    pub redacted: Vec<SecretKind>,
}

/// The prompt as it may leave: secrets blanked unless the policy turned redaction off.
pub fn prepare(policy: &Policy, prompt: &str) -> Outgoing {
    if !policy.redacts() {
        return Outgoing { prompt: prompt.to_string(), redacted: Vec::new() };
    }
    let r = secrets::redact(prompt);
    Outgoing { prompt: r.text, redacted: r.kinds }
}
