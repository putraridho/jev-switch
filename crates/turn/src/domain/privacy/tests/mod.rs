//! Tests for the privacy policy, by subject; shared helpers live here.

mod outgoing;
mod parsing;
mod paths;
mod rules;

use std::path::{Path, PathBuf};

use super::verdict::Verdict;
use super::{Policy, Source};
use jev_shared::domain::path_text::{glob_matches, mentioned_paths};
use jev_shared::ProviderId;

const ROOT: &str = "/work/app";

fn policy(text: &str) -> Policy {
    let (p, errors) = Policy::parse(text, Source::User);
    assert!(errors.is_empty(), "{errors:?}");
    p
}

fn check(p: &Policy, prompt: &str) -> Verdict {
    p.check(Path::new(ROOT), prompt, &[], None)
}
