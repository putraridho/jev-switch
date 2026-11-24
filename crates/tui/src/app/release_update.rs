//! Updating an installed release: download the newest GitHub release's tarball for this
//! platform, check its SHA-256, and replace the installed programs.
//!
//! This is how a jev-code that was *downloaded* (built by the release workflow with
//! `JEV_RELEASE_BUILD` set, installed by `scripts/install-release.sh` or by an earlier
//! `/update`) keeps itself current. A jev-code built from a checkout updates through
//! [`crate::app::self_update`] instead, so people with local commits are never surprised.
//!
//! Nothing is built on the machine. Every program is first staged next to its final place and
//! only then renamed over it, one atomic rename each, so a failed download, a bad checksum or
//! an unwritable folder changes nothing. The running process keeps the old binary until it is
//! restarted. The network is `curl` and the unpacking is `tar`, both on every Linux, macOS and
//! Windows 10+ (its `tar` is bsdtar, which also reads the `.zip` Windows releases are packed in).
//!
//! On Windows a running program's file cannot be overwritten but can be renamed, so each old file
//! is renamed to `.<name>.old` and the new one renamed into its place; the leftovers are removed
//! by the next update.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The GitHub repository the releases live in.
pub const REPO: &str = "putraridho/jev-switch";

/// Names a different release API URL (`file://` allowed): for testing and mirrors.
const API_ENV: &str = "JEV_RELEASE_API";

/// Names the folder holding the installed programs, instead of the running binary's own.
const DIR_ENV: &str = "JEV_INSTALL_DIR";

/// The programs in a release. The archive holds exactly these (see [`Platform::files`]).
pub const PROGRAMS: [&str; 6] = ["jev-code", "jev-ask", "jev-delegate-mcp", "jev-chat", "jev-gateway", "jev-claude"];

/// The mod jev-claude loads, shipped beside the programs under `mods/`.
const MOD: &str = "jev-orchestrate";

/// Bounds on what is downloaded.
const MAX_API_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 4 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 400 * 1024 * 1024;

/// Whether this program is a downloaded release rather than a build from a checkout.
pub fn is_release_build() -> bool {
    option_env!("JEV_RELEASE_BUILD").is_some()
}

/// The platforms releases are built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    LinuxX86_64,
    LinuxAarch64,
    MacosX86_64,
    MacosAarch64,
    WindowsX86_64,
}

/// How a release is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    TarGz,
    Zip,
}

impl Archive {
    fn extension(self) -> &'static str {
        match self {
            Archive::TarGz => "tar.gz",
            Archive::Zip => "zip",
        }
    }
}

impl Platform {
    pub fn detect() -> Option<Platform> {
        Platform::from_parts(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn from_parts(os: &str, arch: &str) -> Option<Platform> {
        match (os, arch) {
            ("linux", "x86_64") => Some(Platform::LinuxX86_64),
            ("linux", "aarch64") => Some(Platform::LinuxAarch64),
            ("macos", "x86_64") => Some(Platform::MacosX86_64),
            ("macos", "aarch64") => Some(Platform::MacosAarch64),
            ("windows", "x86_64") => Some(Platform::WindowsX86_64),
            _ => None,
        }
    }

    /// The name in the tarball: `linux-x86_64`.
    pub fn name(self) -> &'static str {
        match self {
            Platform::LinuxX86_64 => "linux-x86_64",
            Platform::LinuxAarch64 => "linux-aarch64",
            Platform::MacosX86_64 => "macos-x86_64",
            Platform::MacosAarch64 => "macos-aarch64",
            Platform::WindowsX86_64 => "windows-x86_64",
        }
    }

    pub fn archive(self) -> Archive {
        if self == Platform::WindowsX86_64 {
            Archive::Zip
        } else {
            Archive::TarGz
        }
    }

    /// The files of a release as installed beside each other. On Windows the programs are
    /// `.exe` and `jev-claude` is a `.cmd` that runs its PowerShell script.
    pub fn files(self) -> Vec<String> {
        if self != Platform::WindowsX86_64 {
            return PROGRAMS.iter().map(|p| (*p).to_string()).collect();
        }
        let mut files: Vec<String> = PROGRAMS.iter().filter(|p| **p != "jev-claude").map(|p| format!("{p}.exe")).collect();
        files.extend(["jev-claude.cmd".to_string(), "jev-claude.ps1".to_string()]);
        files
    }
}

/// Which URLs a download may use. Releases come over https; only an explicitly named release
/// API (tests, a mirror) may also hand out local files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allow {
    HttpsOnly,
    HttpsOrFile,
}

impl Allow {
    fn protocols(self) -> &'static str {
        match self {
            Allow::HttpsOnly => "=https",
            Allow::HttpsOrFile => "=https,file",
        }
    }

    fn permits(self, url: &str) -> bool {
        url.starts_with("https://") || (self == Allow::HttpsOrFile && url.starts_with("file://"))
    }
}

/// A release that has a build for this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `1.14.0`.
    pub version: String,
    /// `v1.14.0`.
    pub tag: String,
    /// Where the release is described, for the person to read.
    pub page: String,
    pub platform: Platform,
    pub archive_url: String,
    pub checksum_url: String,
    pub allow: Allow,
}

impl Release {
    /// The folder at the top of the tarball.
    fn top_dir(&self) -> String {
        format!("jev-code-{}-{}", self.tag, self.platform.name())
    }

    /// The archive's file name on the release page.
    fn archive_name(&self) -> String {
        format!("{}.{}", self.top_dir(), self.platform.archive().extension())
    }
}

#[derive(serde::Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<ApiAsset>,
}

#[derive(serde::Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

/// A tag that is safe to put in a file name and a URL: `v` and a version of letters, digits and
/// `.-+`. The tag comes from the network, so nothing else is accepted.
fn tag_version(tag: &str) -> Option<&str> {
    let version = tag.strip_prefix('v')?;
    let ok = !version.is_empty() && version.starts_with(|c: char| c.is_ascii_digit()) && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    ok.then_some(version)
}

/// The release API's answer for `/releases/latest`, reduced to the build for `platform`.
pub fn parse_release(json: &str, platform: Platform, allow: Allow) -> Result<Release, String> {
    let api: ApiRelease = serde_json::from_str(json).map_err(|e| format!("the release answer was not understood: {e}"))?;
    let version = tag_version(&api.tag_name).ok_or_else(|| format!("the newest release has an unusable tag ({:?})", api.tag_name))?.to_string();
    let archive = format!("jev-code-{}-{}.{}", api.tag_name, platform.name(), platform.archive().extension());
    let checksum = format!("{archive}.sha256");
    let find = |name: &str| api.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
    let (Some(archive_url), Some(checksum_url)) = (find(&archive), find(&checksum)) else {
        return Err(format!("release {} has no {archive} (with its .sha256) for this platform", api.tag_name));
    };
    if let Some(bad) = [&archive_url, &checksum_url].into_iter().find(|u| !allow.permits(u)) {
        return Err(format!("release {} points at {bad}, which is not an https address", api.tag_name));
    }
    Ok(Release { version, tag: api.tag_name, page: api.html_url, platform, archive_url, checksum_url, allow })
}

/// `curl` one URL: its body, or written to `dest`. Fails on an HTTP error, and stays within
/// `allow` and `max_bytes`, redirects included.
fn fetch(url: &str, dest: Option<&Path>, max_bytes: u64, allow: Allow) -> Result<String, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["--fail", "--silent", "--show-error", "--location", "--connect-timeout", "15", "--max-time", "600"])
        .args(["--proto", allow.protocols(), "--proto-redir", allow.protocols()])
        .args(["--max-filesize", &max_bytes.to_string()])
        .args(["-H", "User-Agent: jev-code-updater", "-H", "Accept: application/vnd.github+json, application/octet-stream"]);
    if let Some(dest) = dest {
        cmd.arg("--output").arg(dest);
    }
    let out = cmd.arg("--").arg(url).stdin(Stdio::null()).output().map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        let why = crate::render::clean_text(&String::from_utf8_lossy(&out.stderr));
        return Err(format!("downloading {url} failed ({}){}", out.status, if why.trim().is_empty() { String::new() } else { format!(": {}", why.trim()) }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The newest release, from `api` (the GitHub API's `latest` for [`REPO`] unless
/// `JEV_RELEASE_API` says otherwise).
pub fn latest() -> Result<Release, String> {
    let platform = Platform::detect().ok_or_else(|| {
        format!(
            "there is no prebuilt jev-code for {}-{}; build it from a checkout with scripts/install.sh",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    match jev_shared::infrastructure::env_file::lookup(API_ENV).filter(|u| !u.trim().is_empty()) {
        Some(url) => latest_from(url.trim(), platform, Allow::HttpsOrFile),
        None => latest_from(&format!("https://api.github.com/repos/{REPO}/releases/latest"), platform, Allow::HttpsOnly),
    }
}

/// [`latest`] against one API address.
pub fn latest_from(api: &str, platform: Platform, allow: Allow) -> Result<Release, String> {
    let body = fetch(api, None, MAX_API_BYTES, allow)?;
    parse_release(&body, platform, allow)
}

/// The folder the programs are installed in: `JEV_INSTALL_DIR`, else where the running
/// `jev-code` really lives (a symlink is followed, and an already-replaced binary still resolves
/// its folder).
pub fn install_dir() -> Result<PathBuf, String> {
    if let Some(dir) = jev_shared::infrastructure::env_file::lookup(DIR_ENV).filter(|d| !d.trim().is_empty()) {
        return Ok(PathBuf::from(dir.trim()));
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot tell where jev-code is installed: {e}"))?;
    resolve_install_dir(&exe)
}

/// The folder holding `exe`, following symlinks when `exe` still exists. When the running binary
/// was already replaced on disk (for instance by an earlier `/update` in the same session, which
/// makes Linux's `/proc/self/exe` end in ` (deleted)`), strip that suffix if a replacement lives
/// there, else canonicalise the parent folder directly.
fn resolve_install_dir(exe: &Path) -> Result<PathBuf, String> {
    if let Ok(real) = exe.canonicalize() {
        return real.parent().map(Path::to_path_buf).ok_or_else(|| "cannot tell where jev-code is installed".to_string());
    }
    if let (Some(parent), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) {
        if let Some(stripped) = name.strip_suffix(" (deleted)") {
            if let Ok(real) = parent.join(stripped).canonicalize() {
                if let Some(dir) = real.parent() {
                    return Ok(dir.to_path_buf());
                }
            }
        }
    }
    let parent = exe.parent().ok_or_else(|| "cannot tell where jev-code is installed".to_string())?;
    parent.canonicalize().map_err(|e| format!("cannot tell where jev-code is installed: {e}"))
}

/// Where `jev-claude` loads the orchestrate mod from.
pub fn mods_dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| Path::new(&h).join(".local/share")))?;
    Some(data.join("jev-router/mods"))
}

/// The hash a `.sha256` file names: its first word, 64 hex digits, lower-cased.
pub fn expected_checksum(text: &str) -> Option<String> {
    let word = text.split_whitespace().next()?;
    (word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit())).then(|| word.to_ascii_lowercase())
}

/// The SHA-256 of a file.
fn sha256_of(file: &Path) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("could not read {} to check it: {e}", file.display()))?;
    Ok(jev_shared::domain::sha256::hex(&bytes))
}

/// A fresh, private folder for the download, removed when dropped.
struct Workdir(PathBuf);

impl Workdir {
    fn create() -> Result<Workdir, String> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("jev-update-{}-{nanos}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        #[cfg(not(unix))]
        let _ = &mut builder;
        builder.create(&path).map_err(|e| format!("could not make a download folder {}: {e}", path.display()))?;
        Ok(Workdir(path))
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A regular file (not a link) at `path`.
fn is_plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn staged_name(file: &str) -> String {
    format!(".{file}.new")
}

fn aside_name(file: &str) -> String {
    format!(".{file}.old")
}

/// Put `stage` in the place of `dest`. On Windows a running program's file cannot be
/// overwritten but can be renamed, so the old one is moved aside first (and put back if the
/// new one cannot take its place); the aside file is removed by the next update.
fn replace_file(stage: &Path, dest: &Path, aside: &Path) -> std::io::Result<()> {
    if !cfg!(windows) {
        return std::fs::rename(stage, dest);
    }
    let _ = std::fs::remove_file(aside);
    let moved = dest.exists();
    if moved {
        std::fs::rename(dest, aside)?;
    }
    match std::fs::rename(stage, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            if moved {
                let _ = std::fs::rename(aside, dest);
            }
            Err(e)
        }
    }
}

/// Download, verify and install `release` into `dir` (programs) and `mods` (the mod), without
/// stopping the gateway. Everything is staged before anything is replaced.
pub fn install(release: &Release, dir: &Path, mods: Option<&Path>, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    let files = release.platform.files();
    // What an earlier update on Windows moved aside.
    for file in &files {
        let _ = std::fs::remove_file(dir.join(aside_name(file)));
    }
    let work = Workdir::create()?;
    let archive = work.0.join(release.archive_name());
    on_line(&format!("downloading {}", release.archive_url));
    fetch(&release.archive_url, Some(&archive), MAX_ARCHIVE_BYTES, release.allow)?;
    let listed = fetch(&release.checksum_url, None, MAX_CHECKSUM_BYTES, release.allow)?;
    let expected = expected_checksum(&listed).ok_or_else(|| format!("{} does not hold a SHA-256", release.checksum_url))?;
    let actual = sha256_of(&archive)?;
    if actual != expected {
        return Err(format!("the download does not match its checksum (expected {expected}, got {actual}); nothing was changed"));
    }
    on_line("checksum matches; unpacking");
    let unpacked = work.0.join("unpacked");
    std::fs::create_dir(&unpacked).map_err(|e| format!("could not make {}: {e}", unpacked.display()))?;
    let mut tar = Command::new("tar");
    // `-z` only for a .tar.gz: bsdtar (Windows) reads a .zip with plain `-x`.
    tar.arg(if release.platform.archive() == Archive::TarGz { "-xzf" } else { "-xf" });
    let tar = tar.arg(&archive).arg("-C").arg(&unpacked).stdin(Stdio::null()).output().map_err(|e| format!("could not run tar: {e}"))?;
    if !tar.status.success() {
        return Err(format!("could not unpack the download ({}): {}", tar.status, crate::render::clean_text(&String::from_utf8_lossy(&tar.stderr)).trim()));
    }
    let root = unpacked.join(release.top_dir());
    if let Some(missing) = files.iter().find(|f| !is_plain_file(&root.join(f))) {
        return Err(format!("the release does not contain {missing}; nothing was changed"));
    }
    let staged: Vec<PathBuf> = files.iter().map(|f| dir.join(staged_name(f))).collect();
    let discard = |staged: &[PathBuf]| staged.iter().for_each(|s| drop(std::fs::remove_file(s)));
    for (file, stage) in files.iter().zip(&staged) {
        if let Err(e) = stage_program(&root.join(file), stage) {
            discard(&staged);
            return Err(format!("could not write {} (is the folder writable? if a package manager owns it, update with that): {e}; nothing was changed", stage.display()));
        }
    }
    // jev-code last, so an interruption leaves the older launcher with its own helpers.
    for (file, stage) in files.iter().zip(&staged).rev() {
        if let Err(e) = replace_file(stage, &dir.join(file), &dir.join(aside_name(file))) {
            discard(&staged);
            return Err(format!("could not replace {file}: {e}; programs replaced before it are already the new version, run /update again"));
        }
        on_line(&format!("installed {file}"));
    }
    match (mods, root.join("mods").join(MOD)) {
        (Some(mods), shipped) if shipped.is_dir() => install_mod(&shipped, mods).map_err(|e| format!("the programs are installed, but the {MOD} mod could not be: {e}"))?,
        (None, _) => on_line("no data folder (HOME is not set): the orchestrate mod was left as it is"),
        (Some(_), _) => {}
    }
    Ok(())
}

fn stage_program(from: &Path, stage: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(stage);
    std::fs::copy(from, stage)?;
    #[cfg(unix)]
    std::fs::set_permissions(stage, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    Ok(())
}

/// Replace `<mods>/jev-orchestrate` with the shipped copy, through a sibling so a failure
/// leaves the old one.
fn install_mod(shipped: &Path, mods: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(mods)?;
    let (new, current) = (mods.join(format!("{MOD}.new")), mods.join(MOD));
    let _ = std::fs::remove_dir_all(&new);
    if let Err(e) = copy_tree(shipped, &new) {
        let _ = std::fs::remove_dir_all(&new);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&current);
    std::fs::rename(&new, &current)
}

/// Stop a running `jev-gateway` so the next `jev-claude` starts the new one.
fn stop_gateway() {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("taskkill");
        c.args(["/F", "/IM", "jev-gateway.exe"]);
        c
    } else {
        let mut c = Command::new("pkill");
        c.args(["-x", "jev-gateway"]);
        c
    };
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Update the installed release: [`install`] into the real places, then stop a running
/// `jev-gateway` so the next `jev-claude` starts the new one. Returns the version installed.
pub fn apply(release: &Release, on_line: &mut dyn FnMut(&str)) -> Result<String, String> {
    let dir = install_dir()?;
    install(release, &dir, mods_dir().as_deref(), on_line)?;
    stop_gateway();
    Ok(release.version.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(tag: &str, assets: &[(&str, &str)]) -> String {
        let assets: Vec<String> = assets.iter().map(|(n, u)| format!(r#"{{"name":"{n}","browser_download_url":"{u}"}}"#)).collect();
        format!(r#"{{"tag_name":"{tag}","html_url":"https://github.com/x/y/releases/tag/{tag}","assets":[{}]}}"#, assets.join(","))
    }

    #[test]
    fn platforms_are_the_four_built() {
        assert_eq!(Platform::from_parts("linux", "x86_64").map(Platform::name), Some("linux-x86_64"));
        assert_eq!(Platform::from_parts("macos", "aarch64").map(Platform::name), Some("macos-aarch64"));
        assert_eq!(Platform::from_parts("windows", "x86_64").map(Platform::name), Some("windows-x86_64"));
        assert_eq!(Platform::from_parts("windows", "aarch64"), None);
        assert_eq!(Platform::from_parts("linux", "riscv64"), None);
    }

    #[test]
    fn windows_releases_are_zips_of_exe_files_with_a_cmd_launcher() {
        assert_eq!(Platform::WindowsX86_64.archive(), Archive::Zip);
        assert_eq!(Platform::LinuxX86_64.archive(), Archive::TarGz);
        let files = Platform::WindowsX86_64.files();
        assert_eq!(files.first().map(String::as_str), Some("jev-code.exe"), "jev-code is first, so it is replaced last");
        assert!(files.contains(&"jev-gateway.exe".to_string()) && files.contains(&"jev-claude.cmd".to_string()) && files.contains(&"jev-claude.ps1".to_string()));
        assert!(!files.contains(&"jev-claude".to_string()), "the bash launcher is not shipped to Windows");
        assert_eq!(Platform::MacosAarch64.files().len(), PROGRAMS.len());
        let json = api(
            "v1.14.0",
            &[("jev-code-v1.14.0-windows-x86_64.zip", "https://h/win.zip"), ("jev-code-v1.14.0-windows-x86_64.zip.sha256", "https://h/win.sha")],
        );
        let r = parse_release(&json, Platform::WindowsX86_64, Allow::HttpsOnly).unwrap();
        assert_eq!((r.archive_name().as_str(), r.archive_url.as_str()), ("jev-code-v1.14.0-windows-x86_64.zip", "https://h/win.zip"));
    }

    #[test]
    fn the_release_for_this_platform_is_picked() {
        let json = api(
            "v1.14.0",
            &[
                ("jev-code-v1.14.0-macos-aarch64.tar.gz", "https://h/mac.tgz"),
                ("jev-code-v1.14.0-linux-x86_64.tar.gz", "https://h/lin.tgz"),
                ("jev-code-v1.14.0-linux-x86_64.tar.gz.sha256", "https://h/lin.sha"),
            ],
        );
        let r = parse_release(&json, Platform::LinuxX86_64, Allow::HttpsOnly).unwrap();
        assert_eq!((r.version.as_str(), r.archive_url.as_str(), r.checksum_url.as_str()), ("1.14.0", "https://h/lin.tgz", "https://h/lin.sha"));
        assert_eq!(r.top_dir(), "jev-code-v1.14.0-linux-x86_64");
        let none = parse_release(&json, Platform::MacosAarch64, Allow::HttpsOnly);
        assert!(none.unwrap_err().contains("no jev-code-v1.14.0-macos-aarch64.tar.gz"), "a build without its checksum is refused");
    }

    #[test]
    fn tags_and_urls_from_the_network_are_vetted() {
        for tag in ["1.14.0", "v", "v../etc", "v1.0/../x", "vx1", "v1 0"] {
            assert!(tag_version(tag).is_none(), "{tag}");
        }
        assert_eq!(tag_version("v1.14.0-rc.1+b2"), Some("1.14.0-rc.1+b2"));
        let json = api("v1.0.0", &[("jev-code-v1.0.0-linux-x86_64.tar.gz", "http://h/a"), ("jev-code-v1.0.0-linux-x86_64.tar.gz.sha256", "https://h/b")]);
        assert!(parse_release(&json, Platform::LinuxX86_64, Allow::HttpsOnly).unwrap_err().contains("not an https address"));
        let local = api("v1.0.0", &[("jev-code-v1.0.0-linux-x86_64.tar.gz", "file:///a"), ("jev-code-v1.0.0-linux-x86_64.tar.gz.sha256", "file:///b")]);
        assert!(parse_release(&local, Platform::LinuxX86_64, Allow::HttpsOnly).is_err(), "files only when the API was named by the person");
        assert!(parse_release(&local, Platform::LinuxX86_64, Allow::HttpsOrFile).is_ok());
        assert!(parse_release("not json", Platform::LinuxX86_64, Allow::HttpsOnly).is_err());
    }

    #[test]
    fn the_checksum_is_the_first_word_in_hex() {
        let hash = "a".repeat(64);
        assert_eq!(expected_checksum(&format!("{}  file.tar.gz\n", hash.to_uppercase())), Some(hash));
        assert_eq!(expected_checksum("short  file"), None);
        assert_eq!(expected_checksum(&format!("{}z", "a".repeat(63))), None);
        assert_eq!(expected_checksum(""), None);
    }

    /// A release built by hand and served from files: the whole download, verify, unpack and
    /// replace path, with a bad checksum and a missing program refused before anything changes.
    #[test]
    fn install_replaces_the_programs_and_refuses_a_bad_download() {
        let root = std::env::temp_dir().join(format!("jev-release-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (serve, bin, mods) = (root.join("serve"), root.join("bin"), root.join("mods"));
        let top = "jev-code-v2.0.0-linux-x86_64";
        let pack = root.join("pack").join(top);
        std::fs::create_dir_all(pack.join("mods").join(MOD)).unwrap();
        std::fs::create_dir_all(&serve).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(pack.join("mods").join(MOD).join("plugin.json"), "new").unwrap();
        for p in PROGRAMS {
            std::fs::write(pack.join(p), format!("new {p}")).unwrap();
            std::fs::write(bin.join(p), format!("old {p}")).unwrap();
        }
        let archive = serve.join(format!("{top}.tar.gz"));
        let ok = |cmd: &mut Command| assert!(cmd.stdin(Stdio::null()).status().unwrap().success());
        ok(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(root.join("pack")).arg(top));
        let hash = sha256_of(&archive).unwrap();
        let sum = serve.join(format!("{top}.tar.gz.sha256"));
        std::fs::write(&sum, format!("{hash}  {top}.tar.gz\n")).unwrap();
        let release = |sum: &Path| Release {
            version: "2.0.0".into(),
            tag: "v2.0.0".into(),
            page: String::new(),
            platform: Platform::LinuxX86_64,
            archive_url: format!("file://{}", archive.display()),
            checksum_url: format!("file://{}", sum.display()),
            allow: Allow::HttpsOrFile,
        };

        // A wrong checksum changes nothing.
        let wrong = serve.join("wrong.sha256");
        std::fs::write(&wrong, format!("{}  x\n", "0".repeat(64))).unwrap();
        let err = install(&release(&wrong), &bin, Some(&mods), &mut |_| {}).unwrap_err();
        assert!(err.contains("does not match its checksum"), "{err}");
        assert_eq!(std::fs::read_to_string(bin.join("jev-code")).unwrap(), "old jev-code");

        // The real one replaces every program, executable, and the mod; no staging file is left.
        let mut lines = Vec::new();
        install(&release(&sum), &bin, Some(&mods), &mut |l| lines.push(l.to_string())).unwrap();
        for p in PROGRAMS {
            assert_eq!(std::fs::read_to_string(bin.join(p)).unwrap(), format!("new {p}"));
            assert!(!bin.join(staged_name(p)).exists());
        }
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(bin.join("jev-code")).unwrap().permissions()) & 0o111, 0o111);
        assert_eq!(std::fs::read_to_string(mods.join(MOD).join("plugin.json")).unwrap(), "new");
        assert!(lines.iter().any(|l| l == "installed jev-code"), "{lines:?}");

        // A release that lacks a program is refused whole.
        std::fs::remove_file(pack.join("jev-chat")).unwrap();
        std::fs::write(bin.join("jev-code"), "marker").unwrap();
        ok(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(root.join("pack")).arg(top));
        std::fs::write(&sum, format!("{}  x\n", sha256_of(&archive).unwrap())).unwrap();
        let err = install(&release(&sum), &bin, Some(&mods), &mut |_| {}).unwrap_err();
        assert!(err.contains("does not contain jev-chat"), "{err}");
        assert_eq!(std::fs::read_to_string(bin.join("jev-code")).unwrap(), "marker");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn latest_reads_the_api_and_a_missing_one_is_an_error() {
        let root = std::env::temp_dir().join(format!("jev-release-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("latest.json");
        std::fs::write(&file, api("v3.1.0", &[("jev-code-v3.1.0-linux-x86_64.tar.gz", "https://h/a"), ("jev-code-v3.1.0-linux-x86_64.tar.gz.sha256", "https://h/b")])).unwrap();
        let url = format!("file://{}", file.display());
        assert_eq!(latest_from(&url, Platform::LinuxX86_64, Allow::HttpsOrFile).unwrap().version, "3.1.0");
        assert!(latest_from(&url, Platform::LinuxX86_64, Allow::HttpsOnly).is_err(), "a file address is refused unless allowed");
        assert!(latest_from(&format!("file://{}/none.json", root.display()), Platform::LinuxX86_64, Allow::HttpsOrFile).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_dir_resolves_after_the_running_binary_was_replaced() {
        let root = std::env::temp_dir().join(format!("jev-install-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let canonical_bin = bin.canonicalize().unwrap();

        // Normal existing binary.
        let exe = bin.join("jev-code");
        std::fs::write(&exe, "v2").unwrap();
        assert_eq!(resolve_install_dir(&exe).unwrap(), canonical_bin);

        // Linux `/proc/self/exe` after an earlier update replaced the binary on disk.
        let deleted = bin.join("jev-code (deleted)");
        assert_eq!(resolve_install_dir(&deleted).unwrap(), canonical_bin);

        // Symlinked install whose target was replaced (Linux appends ` (deleted)`).
        #[cfg(unix)]
        {
            let link_dir = root.join("links");
            std::fs::create_dir_all(&link_dir).unwrap();
            let link = link_dir.join("jev-code");
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            assert_eq!(resolve_install_dir(&link).unwrap(), canonical_bin);
            assert_eq!(resolve_install_dir(&link_dir.join("jev-code (deleted)")).unwrap(), canonical_bin);
        }

        // Unlinked binary with no replacement yet still resolves to its parent directory.
        assert_eq!(resolve_install_dir(&bin.join(".jev-code.old (deleted)")).unwrap(), canonical_bin);

        // A missing parent directory is still an error.
        let missing = root.join("gone").join("jev-code (deleted)");
        assert!(resolve_install_dir(&missing).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }
}
