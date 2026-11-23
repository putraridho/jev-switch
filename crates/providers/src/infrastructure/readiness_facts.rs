//! The machine half of readiness: the stamps that say when a saved verdict stops applying
//! (the CLI's file, the login file's time and size, the folder's trust), and the capability
//! handshake. A handshake runs `--version` and `--help`, or an ACP `initialize`, and reads
//! trust files; it never sends a prompt, so it spends no quota. Nothing read here is a
//! credential's content: only file times and sizes.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use jev_backend::ProviderId;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::domain::handshake::{acp_args, decide_acp, decide_flags, decide_project, required_flags, FolderFact, Handshake};
use crate::domain::readiness::ReadinessKey;
use super::probe_host::{find_binary, home, installed_for_real, probe_command, run_capture};

/// How long one handshake command may take.
const STEP_CAP: Duration = Duration::from_secs(8);

/// The CLI a provider's turns run (DeepSeek is the claude CLI pointed at another API).
pub fn cli_bin(provider: ProviderId) -> String {
    match provider {
        ProviderId::Claude | ProviderId::DeepSeek => "claude".to_string(),
        ProviderId::Codex => "codex".to_string(),
        ProviderId::Gemini => super::backend_factory::gemini_via().to_string(),
        ProviderId::Grok => "grok".to_string(),
    }
}

/// `<modified secs>:<size>` of a file or folder; empty when it is not there.
fn file_stamp(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else { return String::new() };
    let secs = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    format!("{secs}:{}", meta.len())
}

/// Where each CLI keeps its saved login (a file or folder under the home directory).
fn login_path(provider: ProviderId, bin: &str) -> Option<&'static str> {
    match (provider, bin) {
        (ProviderId::Claude, _) => Some(".claude/.credentials.json"),
        (ProviderId::Codex, _) => Some(".codex/auth.json"),
        (ProviderId::Gemini, "agy") => Some(".gemini/antigravity-cli"),
        (ProviderId::Gemini, _) => Some(".gemini/oauth_creds.json"),
        (ProviderId::Grok, _) => Some(".grok/auth.json"),
        (ProviderId::DeepSeek, _) => None,
    }
}

/// The CLI and the account of `provider`, for [`ReadinessKey`]; the backend is filled in by
/// the caller, who knows how the CLI is funded. A login kept somewhere this cannot see (the
/// macOS keychain, an API key) leaves the account empty: a verdict then ends by its expiry
/// or `/providers refresh`, not by the account changing.
pub fn key_of(provider: ProviderId) -> ReadinessKey {
    let bin = cli_bin(provider);
    let cli = find_binary(&bin).map_or(String::new(), |p| format!("{}:{}", p.display(), file_stamp(&p)));
    let account = login_path(provider, &bin).and_then(|rel| home().map(|h| file_stamp(&h.join(rel)))).unwrap_or_default();
    ReadinessKey { cli, backend: String::new(), account }
}

/// What `trustedFolders.json` (Gemini CLI and Antigravity) says about `dir`: the folder or an
/// ancestor trusted, the folder refused, or nothing. Written from the file's documented
/// shape (folder path to `TRUST_FOLDER`, `TRUST_PARENT` or `DO_NOT_TRUST`), not checked
/// against every release.
pub fn trust_in(json: &str, dir: &Path) -> FolderFact {
    let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(json) else { return FolderFact::Unknown };
    let mut trusted = false;
    for (path, rule) in &map {
        let at = Path::new(path);
        match rule.as_str() {
            "DO_NOT_TRUST" if dir == at => return FolderFact::Untrusted,
            "TRUST_FOLDER" if dir.starts_with(at) => trusted = true,
            "TRUST_PARENT" if at.parent().is_some_and(|parent| dir.starts_with(parent)) => trusted = true,
            _ => {}
        }
    }
    if trusted {
        FolderFact::Trusted
    } else {
        FolderFact::Unknown
    }
}

/// What the machine shows about `dir` for the CLIs that have a folder rule.
pub fn folder_fact(provider: ProviderId, dir: &Path) -> FolderFact {
    match provider {
        ProviderId::Codex => {
            if dir.ancestors().any(|a| a.join(".git").exists()) {
                FolderFact::InGitRepo
            } else {
                FolderFact::NotInGitRepo
            }
        }
        ProviderId::Gemini => super::probe_host::read_home_file(".gemini/trustedFolders.json").map_or(FolderFact::Unknown, |text| trust_in(&text, dir)),
        _ => FolderFact::NotApplicable,
    }
}

/// The project part of the saved key: the folder and what its rule says now, so a trust
/// decision made later gives a new key.
pub fn project_key(provider: ProviderId, dir: &Path) -> String {
    format!("{}#{}", dir.display(), folder_fact(provider, dir).word())
}

async fn first_line(bin: &str, args: &[&str], dir: &Path, request: &str) -> Option<String> {
    let mut cmd = probe_command(bin, args);
    cmd.current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(request.as_bytes()).await.ok()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    let answer = tokio::time::timeout(STEP_CAP, lines.next_line()).await.ok()?.ok()??;
    drop(stdin);
    let _ = child.kill().await;
    Some(answer)
}

async fn capture(bin: &str, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(STEP_CAP, run_capture(bin, args)).await.ok()??;
    out.ok.then_some(out.text)
}

/// The capability handshake of one provider in `dir`. A CLI that is not really installed
/// (or is a lazy wrapper whose first run installs it) is not run at all.
pub async fn handshake(provider: ProviderId, dir: PathBuf) -> Handshake {
    let bin = cli_bin(provider);
    if !installed_for_real(&bin) {
        return Handshake::unverified("the CLI is not installed (or is a lazy wrapper that is not run here)");
    }
    let protocol = match acp_args(provider, &bin) {
        Some(args) => decide_acp(first_line(&bin, args, &dir, &jev_backend::infrastructure::gemini::acp::initialize_request()).await.as_deref()),
        None => {
            let version = capture(&bin, &["--version"]).await.map(|t| t.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim().to_string());
            let help = capture(&bin, &["--help"]).await;
            decide_flags(version.as_deref(), help.as_deref(), required_flags(provider, &bin))
        }
    };
    Handshake { protocol, project: decide_project(provider, folder_fact(provider, &dir)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_folders_decide_by_the_folder_and_its_ancestors() {
        let json = r#"{"/work/app":"TRUST_FOLDER","/work/secret":"DO_NOT_TRUST","/work/lib/x":"TRUST_PARENT"}"#;
        assert_eq!(trust_in(json, Path::new("/work/app/sub")), FolderFact::Trusted);
        assert_eq!(trust_in(json, Path::new("/work/secret")), FolderFact::Untrusted);
        assert_eq!(trust_in(json, Path::new("/work/lib/y")), FolderFact::Trusted, "the parent of x is trusted");
        assert_eq!(trust_in(json, Path::new("/elsewhere")), FolderFact::Unknown);
        assert_eq!(trust_in("nope", Path::new("/work/app")), FolderFact::Unknown);
    }

    #[test]
    fn a_codex_folder_inside_a_repo_passes_and_one_outside_does_not() {
        let repo = jev_testkit::scratch_dir::scratch_dir("readiness-facts", "repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        assert_eq!(folder_fact(ProviderId::Codex, &repo.join("sub")), FolderFact::InGitRepo);
        assert_eq!(folder_fact(ProviderId::Claude, &repo), FolderFact::NotApplicable);
        assert!(project_key(ProviderId::Codex, &repo).ends_with("#git"));
    }

    #[test]
    fn a_stamp_is_a_time_and_a_size_and_empty_for_a_missing_file() {
        let d = jev_testkit::scratch_dir::scratch_dir("readiness-facts", "stamp");
        std::fs::write(d.join("auth.json"), "secret").unwrap();
        let stamp = file_stamp(&d.join("auth.json"));
        assert!(stamp.ends_with(":6") && !stamp.contains("secret"), "{stamp}");
        assert_eq!(file_stamp(&d.join("missing")), "");
    }
}
