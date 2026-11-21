use crate::approval::domain::mode::Mode;
use crate::tui::render::{display_width, Style};

/// The roles colour plays on this screen, in one place. Nothing here is colour alone: each
/// role also has a glyph or a word.
pub(super) struct Theme(pub(super) Style);

impl Theme {
    pub(super) fn provider_theme(style: Style) -> crate::tui::render::Theme {
        crate::tui::render::Theme::new(style)
    }
    pub(super) fn accent(&self, s: &str) -> String {
        self.0.accent(s)
    }
    pub(super) fn muted(&self, s: &str) -> String {
        self.0.dim(s)
    }
    pub(super) fn strong(&self, s: &str) -> String {
        self.0.bold(s)
    }
    pub(super) fn warning(&self, s: &str) -> String {
        self.0.yellow(s)
    }
    /// The input box's edge: a control's boundary, so it stays visible.
    pub(super) fn field(&self, s: &str) -> String {
        self.0.field(s)
    }
    /// A raised surface (the menu's selected row, an approval card): a step off the background.
    pub(super) fn raised(&self, s: &str) -> String {
        self.0.band(s)
    }
    pub(super) fn selected(&self, s: &str) -> String {
        self.0.bold(s)
    }
    /// The mode's name, coloured by how much it lets the agent do.
    pub(super) fn mode(&self, m: Mode) -> String {
        let text = format!("{} mode", m.name());
        match m {
            Mode::Auto => self.0.yellow(&text),
            Mode::Plan => self.0.cyan(&text),
            Mode::Edit => self.0.green(&text),
            Mode::Ask => text,
        }
    }
    /// Key hints on one row of `width`, `  Enter:send │ Shift+Tab:mode`: the key bright, what it
    /// does dim. The ones that do not fit are left off whole, never cut in half.
    pub(super) fn keys(&self, pairs: &[(&str, &str)], width: usize) -> String {
        let sep = self.muted(" │ ");
        let mut row = String::from(" ");
        for (i, (k, does)) in pairs.iter().enumerate() {
            let hint = format!("{}{}", self.strong(k), self.muted(&format!(":{does}")));
            let next = if i == 0 { format!("{row} {hint}") } else { format!("{row}{sep}{hint}") };
            if display_width(&next) > width {
                break;
            }
            row = next;
        }
        row
    }
}
