//! What a user changed about a provider's look, parsed from `provider.field=value` lines.

use std::collections::BTreeMap;

use super::color::Rgb;
use super::Identity;
use crate::render::width::display_width;

/// What a user changed about one provider's look; anything left `None` keeps the built-in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityPatch {
    pub accent: Option<Rgb>,
    pub glyph: Option<char>,
    pub badge: Option<String>,
}

/// Why one override line was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideError {
    /// Not `provider.field=value`.
    Malformed(String),
    UnknownField(String),
    BadColour(String),
    /// A glyph must be one character that takes one column.
    BadGlyph(String),
    /// A badge is 1 to 8 columns.
    BadBadge(String),
}

impl std::fmt::Display for OverrideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(l) => write!(f, "`{l}` is not provider.field=value"),
            Self::UnknownField(x) => write!(f, "unknown field `{x}` (accent, glyph, badge)"),
            Self::BadColour(v) => write!(f, "`{v}` is not a #rrggbb colour"),
            Self::BadGlyph(v) => write!(f, "`{v}` is not a single one-column character"),
            Self::BadBadge(v) => write!(f, "`{v}` is not a badge of 1 to 8 columns"),
        }
    }
}

/// Per-provider overrides: `claude.accent=#d97706`, `codex.glyph=*`, `gemini.badge=agy`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeOverrides {
    patches: BTreeMap<String, IdentityPatch>,
}

impl ThemeOverrides {
    /// Parse override lines separated by newlines or `;`. Blank lines and `#` comment lines
    /// are skipped; a bad line is reported and the others still apply.
    pub fn parse(text: &str) -> (Self, Vec<OverrideError>) {
        let mut out = Self::default();
        let mut errors = Vec::new();
        for line in text.split(['\n', ';']).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Err(e) = out.apply_line(line) {
                errors.push(e);
            }
        }
        (out, errors)
    }

    fn apply_line(&mut self, line: &str) -> Result<(), OverrideError> {
        let malformed = || OverrideError::Malformed(line.to_string());
        let (key, value) = line.split_once('=').ok_or_else(malformed)?;
        let (provider, field) = key.trim().split_once('.').ok_or_else(malformed)?;
        let value = value.trim();
        let patch = self.patches.entry(provider.trim().to_ascii_lowercase()).or_default();
        match field.trim() {
            "accent" => patch.accent = Some(Rgb::parse_hex(value).ok_or_else(|| OverrideError::BadColour(value.into()))?),
            "glyph" => {
                let mut cs = value.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) if display_width(&c.to_string()) == 1 => patch.glyph = Some(c),
                    _ => return Err(OverrideError::BadGlyph(value.into())),
                }
            }
            "badge" => {
                let w = display_width(value);
                if w == 0 || w > 8 || value.chars().any(char::is_control) {
                    return Err(OverrideError::BadBadge(value.into()));
                }
                patch.badge = Some(value.to_string());
            }
            other => return Err(OverrideError::UnknownField(other.into())),
        }
        Ok(())
    }

    /// Layer `other` over `self` (what `other` sets wins).
    pub fn merged(mut self, other: &Self) -> Self {
        for (provider, p) in &other.patches {
            let mine = self.patches.entry(provider.clone()).or_default();
            mine.accent = p.accent.or(mine.accent);
            mine.glyph = p.glyph.or(mine.glyph);
            mine.badge = p.badge.clone().or_else(|| mine.badge.take());
        }
        self
    }

    pub(super) fn apply(&self, provider: &str, mut base: Identity) -> Identity {
        if let Some(p) = self.patches.get(&provider.to_ascii_lowercase()) {
            base.accent = p.accent.unwrap_or(base.accent);
            base.glyph = p.glyph.unwrap_or(base.glyph);
            if let Some(b) = &p.badge {
                base.badge = b.clone();
            }
        }
        base
    }
}
