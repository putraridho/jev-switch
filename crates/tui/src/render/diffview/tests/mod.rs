use super::*;
use jev_changes::domain::diff_parse::parse;
use crate::render::strip_ansi;
use crate::render::test_facts::*;
use crate::render::width::display_width;

mod fold;
mod split;
mod unified;
mod word;

pub(super) const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -9,4 +9,5 @@ fn main\n let a = 1;\n-let total = price * 3;\n+let total = price * quantity;\n+let extra = 0;\n done();\n";

pub(super) fn opts(width: usize, layout: Layout) -> DiffOpts {
    DiffOpts {
        width,
        layout,
        fold: Fold::Open,
    }
}
