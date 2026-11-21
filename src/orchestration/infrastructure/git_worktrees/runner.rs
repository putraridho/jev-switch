//! The one way to run git: `git -C dir args`, with an optional stdin.

use std::io::Write as _;
use std::path::Path;
use std::process::{ Command, Stdio };

use anyhow::{ bail, Context, Result };

/// Run `git -C dir args`, optionally feeding `stdin`; stdout on success, a message with
/// git's own words on failure.
pub(super) fn git(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(crate::shared::infrastructure::git_guard::no_repo_programs(dir))
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run git (is it installed?)")?;
    // A separate thread, so a big patch cannot deadlock against git printing before it has read it all.
    let feeder = stdin.zip(child.stdin.take()).map(|(text, mut pipe)| {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        })
    });
    let out = child.wait_with_output().context("waiting for git")?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("git {} failed: {}", args.first().unwrap_or(&""), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
