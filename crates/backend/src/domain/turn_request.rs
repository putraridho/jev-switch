//! Everything a backend needs to run one turn.

use std::path::PathBuf;

/// Everything a backend needs to run one turn.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub dir: PathBuf,
    pub model: String,
    pub prompt: String,
    /// Images to send with the prompt (absolute paths); see [`jev_turn::application::attach`].
    pub images: Vec<PathBuf>,
    pub session_id: Option<String>,
    /// What the turn may do without asking; used by Claude Code, DeepSeek and Antigravity.
    pub mode: jev_approval::domain::mode::Mode,
    /// Whether the person trusted this project's own Claude Code settings (its hooks and
    /// permissions); only Claude Code reads them. Decided by the caller, see
    /// [`jev_approval::application::extension_hooks::trust::project_settings_trusted`].
    pub trust_project_settings: bool,
}

/// The prompt with the attached images listed by path, for CLIs that take no image input
/// but open files themselves (Gemini, Antigravity, Grok).
pub fn prompt_with_images(req: &TurnRequest) -> String {
    if req.images.is_empty() {
        return req.prompt.clone();
    }
    let paths: Vec<String> = req.images.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    format!("{}\n\nAttached images (open them from these paths): {}", req.prompt, paths.join(", "))
}
