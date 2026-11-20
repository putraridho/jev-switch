//! Fixtures shared by the tests of several formatters.

use crate::backend::ErrorKind;

use crate::tui::render::*;

    pub(super) const ON: Style = Style { enabled: true, terminal: true };
    pub(super) const OFF: Style = Style { enabled: false, terminal: false };

    pub(super) fn no_esc(s: &str) {
        assert!(!s.contains('\x1b'), "escape bytes in {s:?}");
    }

    pub(super) fn lines(s: &str) -> Vec<&str> {
        s.split('\n').collect()
    }

    pub(super) fn quota() -> ErrorKind {
        ErrorKind::Quota { resets_at: Some("5pm".into()) }
    }

    pub(super) fn all_kinds() -> Vec<ErrorKind> {
        vec![
            quota(),
            ErrorKind::Quota { resets_at: None },
            ErrorKind::NotEntitled,
            ErrorKind::Auth,
            ErrorKind::Empty,
            ErrorKind::Stalled,
            ErrorKind::UnusableHere,
            ErrorKind::Other,
        ]
    }

    pub(super) fn rows() -> Vec<AvailRow> {
        let mut claude = AvailRow::new("claude", ProviderStatus::Ready);
        claude.plan = Some("max_5x".into());
        claude.cooldown = Some("2h 10m".into());
        let mut codex = AvailRow::new("codex", ProviderStatus::NotLoggedIn);
        codex.detail = "run codex login".into();
        let gemini = AvailRow::new("gemini", ProviderStatus::NotInstalled);
        let mut deepseek = AvailRow::new("deepseek", ProviderStatus::NoKey);
        deepseek.verified = true;
        let mut grok = AvailRow::new("grok", ProviderStatus::Unknown);
        grok.verified = false;
        vec![claude, codex, gemini, deepseek, grok]
    }

    pub(super) fn facts() -> SummaryFacts {
        SummaryFacts {
            ok: true,
            provider: "codex".into(),
            model: "gpt-6-luna".into(),
            elapsed_ms: 14_000,
            in_tokens: 45_000,
            cached_tokens: 40_000,
            out_tokens: 189,
            plan: Some(("claude".into(), 0.32)),
            files_changed: None,
            ..Default::default()
        }
    }

    pub(super) fn fc(status: char, path: &str, added: u32, removed: u32) -> FileChange {
        FileChange { status, path: path.into(), added, removed }
    }

    pub(super) fn item(sev: Severity, title: &str, detail: &str, fix: &[&str]) -> DoctorItem {
        DoctorItem { severity: sev, title: title.into(), detail: detail.into(), fix: fix.iter().map(|s| s.to_string()).collect() }
    }
