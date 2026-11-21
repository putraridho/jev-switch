//! Tests of the approval rules, one file per subject; the shared fakes are here.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::approval::infrastructure::tool_input_wire::tool_input_from_json;

#[allow(unused_imports)]
use crate::approval::domain::{
    always_plan::*,
    bash_rule::*,
    command_class::*,
    grantable::*,
    permission_request::*,
    permission_rule::*,
    refusal::*,
    review::*,
    rule_policy::*,
    settings_document::*,
    shell_words::*,
    tool_input::*,
    visible_text::*,
};

fn project() -> &'static Path {
    Path::new("/work/app")
}

/// The project, a home folder `/home/u` and the temp folder `/tmp`.
fn places() -> Places {
    places_with(Some("/home/u"))
}

fn places_with(home: Option<&str>) -> Places {
    Places { project: project().to_path_buf(), home: home.map(PathBuf::from), temp: PathBuf::from("/tmp") }
}

fn tmp(name: &str) -> PathBuf {
    crate::testkit::scratch_dir::scratch_dir("perm", name)
}

fn bash(command: &str) -> Value {
    json!({ "command": command })
}

fn rule(command: &str) -> Result<String, Refusal> {
    rule_for("Bash", &tool_input_from_json(&bash(command)), None, &places())
}

fn lines_of(tool: &str, input: Value) -> Vec<String> {
    review(tool, &tool_input_from_json(&input), None).lines
}

mod always_plan;
mod bash_rule;
mod grantable;
mod permission_rule;
mod review;
mod rule_policy;
