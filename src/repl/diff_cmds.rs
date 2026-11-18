//! `/diff` (last turn, one earlier turn, or everything; per-hunk review; patch export)
//! and `/rewind`. The wording and layout come from [`crate::uifmt`], the arguments and
//! the git work from [`crate::diffcmd`].

use super::{page, Io, Repl};
use crate::changes::{self, UndoError};
use crate::diffcmd::{self, DiffArgs};
use crate::diffparse::{self, FileDiff};
use crate::terminal;
use crate::uifmt::{self, DiffOpts, Fold};
use crate::views;

impl Repl {
    fn say_dim(&self, text: &str) {
        let style = self.out_style();
        self.console.out(&format!("  {}", style.dim(text)));
    }

    /// `/diff [options] [file]`.
    pub(super) fn diff(&mut self, rest: Option<&str>, io: &mut Io<'_>) {
        let style = self.out_style();
        let args = match diffcmd::parse_args(rest.unwrap_or("")) {
            Ok(a) => a,
            Err(e) => return self.console.out(&format!("{} {e}", style.yellow("⚠"))),
        };
        if args.list {
            return self.page_out(&diffcmd::checkpoint_list(self.engine.change_history(), style));
        }
        let summary = match diffcmd::summary_for(args.scope, self.engine.last_changes(), self.engine.change_history()) {
            Ok(s) => s,
            Err(why) => return self.say_dim(&format!("No changes to show: {why}.")),
        };
        let text = changes::diff_text(self.engine.dir(), &summary, args.path.as_deref()).unwrap_or_default();
        let files = diffparse::parse(&text);
        if files.is_empty() {
            return match &args.path {
                Some(p) => self.say_dim(&format!("No changes to show for {p}: it was not touched (or this folder is not a git repository).")),
                None => self.say_dim("No differences to show."),
            };
        }
        if let Some(dest) = &args.export {
            return match diffcmd::export_patch(self.engine.dir(), &files, dest) {
                Ok(p) => self.console.out(&format!("{} patch written to {} (apply it with `git apply`)", style.green("✓"), p.display())),
                Err(e) => self.console.out(&uifmt::problem_block("Patch not written", &[e], &[], style)),
            };
        }
        let opts = self.diff_opts(&args);
        if args.review {
            let turns = self.engine.change_history().len();
            let newest_only = match args.scope {
                diffcmd::Scope::Last => true,
                diffcmd::Scope::Turn(n) => n == turns,
                diffcmd::Scope::All => turns <= 1,
            };
            return self.review_hunks(&files, &opts, newest_only, io);
        }
        let rendered = uifmt::render_diff(&files, &opts, style);
        if self.paging && views::should_page(&rendered, terminal::rows(), true) && self.console.with_terminal(|| page(&rendered)) {
            return;
        }
        self.console.out(&rendered);
        self.console.out("");
    }

    fn diff_opts(&self, args: &DiffArgs) -> DiffOpts {
        let mut opts = DiffOpts::new(self.console.width());
        opts.layout = args.layout;
        opts.fold = args.fold.unwrap_or(Fold::After(uifmt::FOLD_AFTER));
        opts
    }

    /// Show each hunk and ask whether to keep it; a "yes" to reverting takes it back out
    /// of the file, leaving the other hunks (per-hunk undo).
    fn review_hunks(&mut self, files: &[FileDiff], opts: &DiffOpts, newest_only: bool, io: &mut Io<'_>) {
        let style = self.out_style();
        let Some(confirm) = io.confirm.as_mut() else {
            self.console.out(&format!("{} Review needs a terminal to ask in. Run /diff --review in an interactive session.", style.yellow("⚠")));
            return;
        };
        let opts = DiffOpts { fold: Fold::Open, ..*opts };
        let (mut kept, mut reverted) = (0, 0);
        for file in files {
            // Reverting a hunk does not move the others: git finds them by their context.
            for index in 0..file.hunks.len() {
                let Some(view) = uifmt::render_hunk(file, index, &opts, style) else { continue };
                self.console.out(&view);
                if !confirm.confirm("Revert this hunk? [y/N] ") {
                    kept += 1;
                    continue;
                }
                match diffcmd::revert_hunk(self.engine.dir(), file, index) {
                    Ok(()) => {
                        reverted += 1;
                        self.console.out(&format!("{} hunk reverted", style.green("✓")));
                    }
                    Err(e) => {
                        kept += 1;
                        self.console.out(&uifmt::problem_block("Hunk not reverted", &[e], &[], style));
                    }
                }
            }
        }
        self.console.out(&format!("{} review done: {kept} kept, {reverted} reverted", style.green("✓")));
        if reverted > 0 {
            // The newest turn now ends where the tree is, so /undo and /rewind carry on. An
            // older turn's hunk is not covered: that turn's end no longer matches the files.
            let adopted = self.engine.adopt_working_tree();
            if !adopted || !newest_only {
                self.say_dim("A file you reverted hunks of no longer matches how an earlier turn left it, so /undo and /rewind may refuse it.");
            }
        }
    }

    /// `/rewind [turn]`: list the turns, or undo every turn from `turn` on.
    pub(super) fn rewind(&mut self, turn: Option<usize>, io: &mut Io<'_>) {
        let style = self.out_style();
        let history = self.engine.change_history();
        let Some(number) = turn else {
            self.page_out(&diffcmd::checkpoint_list(history, style));
            if !history.is_empty() {
                self.say_dim("/rewind <number> puts the files back to before that turn.");
            }
            return;
        };
        if number > history.len() {
            return self.say_dim(&format!("There is no turn {number}: this session has {} turn(s) that changed files (see /rewind).", history.len()));
        }
        let turns = history.len() - number + 1;
        let noun = if turns == 1 { "turn" } else { "turns" };
        let files: Vec<String> = history[number - 1..].iter().flat_map(|s| s.files.iter().map(|f| f.path.clone())).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        self.console.out(&format!("  Would undo {turns} {noun}, touching: {}", files.join(", ")));
        let Some(confirm) = io.confirm.as_mut() else {
            self.console.out(&format!("{} Not rewound: asking needs a terminal. Run /rewind in an interactive session.", style.yellow("⚠")));
            return;
        };
        if !confirm.confirm(&format!("Rewind to before turn {number}? [y/N] ")) {
            return self.say_dim("Left as it is.");
        }
        match self.engine.rewind_to(number) {
            Ok(done) => self.console.out(&format!("{} rewound: {} turn(s) undone, {} file(s) back as they were before turn {number}", style.green("✓"), done.len(), files.len())),
            Err((done, e)) => {
                let how_far = format!("{} turn(s) were undone before this stopped", done.len());
                let steps = ["Look at what differs: `/diff`".to_string()];
                let why = match &e {
                    UndoError::ChangedSince(changed) => format!("these files changed since the turn: {}", changed.join(", ")),
                    other => other.to_string(),
                };
                self.console.out(&uifmt::problem_block("Not fully rewound", &[why, how_far], &steps, style));
            }
        }
    }
}
