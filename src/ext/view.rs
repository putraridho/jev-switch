//! The wording of `/extensions`, `/recipe`, `/hooks` and `jev-code ext list`.

use super::hooks::{HookDef, Hooks};
use super::library::{Entry, Kind, Library};
use super::market::{Listing, State};
use crate::permissions::visible;
use crate::uifmt::Style;

fn entry_row(e: &Entry, style: Style) -> String {
    let steps = if e.kind == Kind::Recipe { format!(" · {} steps", e.steps.len()) } else { String::new() };
    let about = if e.meta.description.is_empty() { String::new() } else { format!("  {}", e.meta.description) };
    format!("  {}{about} {}", style.cyan(&e.usage()), style.dim(&format!("({}{steps})", e.origin)))
}

/// Commands, skills and recipes found, then what was skipped.
pub fn library_text(lib: &Library, style: Style) -> String {
    let mut lines = Vec::new();
    for kind in Kind::ALL {
        let entries: Vec<&Entry> = lib.of_kind(kind).collect();
        lines.push(style.bold(&format!("{}s", kind.noun())) + &style.dim(&format!("  .jev/{}/", kind.dir())));
        if entries.is_empty() {
            lines.push(style.dim("  none"));
        }
        lines.extend(entries.into_iter().map(|e| entry_row(e, style)));
    }
    for problem in &lib.problems {
        lines.push(format!("{} {problem}", style.yellow("⚠")));
    }
    lines.join("\n")
}

/// The installed extensions with whether their files still match.
pub fn installed_text(listing: &[Listing], style: Style) -> String {
    if listing.is_empty() {
        return style.dim("  no extensions installed (jev-code ext install <folder>)");
    }
    listing
        .iter()
        .map(|l| {
            let state = match &l.state {
                State::Verified => style.green("verified"),
                State::Changed(files) => style.yellow(&format!("changed: {}", files.join(", "))),
            };
            format!("  {} {}  {state}", style.cyan(&l.name), l.version)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `/recipe`: the recipes, and how to run one.
pub fn recipes_text(lib: &Library, style: Style) -> String {
    let rows: Vec<String> = lib.of_kind(Kind::Recipe).map(|e| entry_row(e, style)).collect();
    if rows.is_empty() {
        return format!("{}\n{}", style.dim("  no recipes yet"), style.dim("  write one in .jev/recipes/<name>.md: a `## step` heading per turn"));
    }
    format!("{}\n{}", rows.join("\n"), style.dim("  run one with /recipe <name> [arguments]"))
}

/// `/hooks`.
pub fn hooks_text(hooks: &Hooks, style: Style) -> String {
    let mut lines: Vec<String> = hooks
        .defs()
        .iter()
        .map(|d| format!("  {} {}  {}", style.cyan(d.event.as_str()), visible(d.tool.as_deref().unwrap_or("*")), style.dim(&visible(&d.run))))
        .collect();
    if lines.is_empty() {
        lines.push(style.dim("  no hooks (.jev/hooks.json or the config folder's hooks.json)"));
    }
    if hooks.skipped_untrusted > 0 {
        lines.push(format!("{} {} hook(s) in this project are off until you trust it: /hooks trust", style.yellow("⚠"), hooks.skipped_untrusted));
        lines.extend(hooks.untrusted_defs().iter().map(|d| {
            format!("  {} {}  {}", style.cyan(d.event.as_str()), visible(d.tool.as_deref().unwrap_or("*")), visible(&d.run))
        }));
    }
    for problem in &hooks.problems {
        lines.push(format!("{} {problem}", style.yellow("⚠")));
    }
    lines.join("\n")
}

/// The project commands approved by `/hooks trust`.
pub fn approved_hooks_text(defs: &[HookDef], style: Style) -> String {
    let mut lines = vec!["  project hooks approved:".to_string()];
    if defs.is_empty() {
        lines.push(style.dim("  none"));
    }
    lines.extend(defs.iter().map(|d| {
        format!("  {} {}  {}", style.cyan(d.event.as_str()), visible(d.tool.as_deref().unwrap_or("*")), visible(&d.run))
    }));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::library::Origin;
    use crate::ext::frontmatter::Meta;
    use std::path::PathBuf;

    #[test]
    fn rows_name_the_usage_the_origin_and_the_steps() {
        let lib = Library {
            entries: vec![Entry { name: "ship".into(), kind: Kind::Recipe, origin: Origin::User, meta: Meta { description: "Ship".into(), hint: "[tag]".into(), mode: None }, steps: vec![] }],
            problems: vec!["x.md: no body".into()],
        };
        let text = library_text(&lib, Style::off());
        assert!(text.contains("/ship [tag]  Ship (user · 0 steps)") && text.contains("⚠ x.md: no body"), "{text}");
        assert!(recipes_text(&Library::default(), Style::off()).contains("no recipes yet"));
        assert!(recipes_text(&lib, Style::off()).contains("/recipe <name>"));
    }

    #[test]
    fn installed_extensions_show_their_checksum_state() {
        let ok = Listing { name: "a".into(), version: "1".into(), source: PathBuf::new(), state: State::Verified };
        let bad = Listing { state: State::Changed(vec!["commands/x.md".into()]), name: "b".into(), ..ok.clone() };
        let text = installed_text(&[ok, bad], Style::off());
        assert!(text.contains("a 1  verified") && text.contains("changed: commands/x.md"), "{text}");
        assert!(installed_text(&[], Style::off()).contains("no extensions installed"));
    }

    #[test]
    fn hooks_text_lists_and_warns() {
        let text = hooks_text(&Hooks::none(std::path::Path::new("/p")), Style::off());
        assert!(text.contains("no hooks"));
    }

    #[test]
    fn untrusted_and_approved_commands_show_controls_visibly() {
        use crate::ext::Places;
        let dir = std::env::temp_dir().join(format!("jev-hooks-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".jev")).unwrap();
        std::fs::write(dir.join(".jev/hooks.json"), r#"{"hooks":[{"event":"turn_start","run":"echo first\n\u001bsecond"}]}"#).unwrap();
        let places = Places { project: dir.clone(), user: Some(dir.join("cfg")) };
        let loaded = Hooks::load(&places);
        let text = hooks_text(&loaded, Style::off());
        assert!(text.contains("turn_start") && text.contains("echo first␊␛second"), "{text}");
        let (_, defs) = crate::ext::hooks::trust_project_with_preview(places.user.as_deref().unwrap(), &dir).unwrap();
        let approved = approved_hooks_text(&defs, Style::off());
        assert!(approved.contains("turn_start") && approved.contains("echo first␊␛second"), "{approved}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
