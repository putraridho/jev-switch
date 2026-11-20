//! The wording of `/extensions`, `/recipe`, `/hooks` and `jev-code ext list`.

use crate::approval::application::extension_hooks::{HookDef, Hooks};
use crate::approval::application::extension_library::{Entry, Kind, Library};
use crate::approval::application::extension_market::{Listing, State};
use crate::approval::domain::rule_policy::visible;
use crate::tui::render::Style;

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

/// `/skill`: the skills as an aligned table, and how to use them.
pub fn skills_text(lib: &Library, style: Style) -> String {
    let skills: Vec<&Entry> = lib.of_kind(Kind::Skill).collect();
    if skills.is_empty() {
        return [
            style.dim("  no skills yet"),
            style.dim("  a skill is a saved know-how the agent follows when you call it by name"),
            format!("  start one: {}", style.cyan("/skill new <name> [what it is for]")),
            style.dim("  or add .jev/skills/<name>/SKILL.md (also read from ~/.config/jev-code/skills)"),
        ]
        .join("\n");
    }
    let width = skills.iter().map(|e| e.usage().chars().count()).max().unwrap_or(0);
    let mut lines = vec![style.bold(&format!("skills ({})", skills.len()))];
    for e in &skills {
        let usage = e.usage();
        let pad = " ".repeat(width - usage.chars().count());
        let about = if e.meta.description.is_empty() { style.dim("no description") } else { e.meta.description.clone() };
        lines.push(format!("  {}{pad}  {about} {}", style.cyan(&usage), style.dim(&format!("({})", e.origin))));
    }
    for problem in lib.problems.iter().filter(|p| p.contains("SKILL.md")) {
        lines.push(format!("{} {problem}", style.yellow("⚠")));
    }
    lines.push(String::new());
    lines.push(style.dim("  run one: /skill <name> [arguments] (or just /<name>) · read one: /skill show <name> · start one: /skill new <name>"));
    lines.join("\n")
}

/// `/skill show <name>`: what a skill says and where it lives, so it can be read before it runs.
pub fn skill_text(e: &Entry, style: Style) -> String {
    let mut lines = vec![format!("{}  {}", style.bold(&e.usage()), style.dim(&format!("({})", e.origin)))];
    if !e.meta.description.is_empty() {
        lines.push(format!("  {}", e.meta.description));
    }
    let mut facts = vec![e.path.display().to_string()];
    if let Some(mode) = e.meta.mode {
        facts.push(format!("runs in {mode} mode"));
    }
    lines.push(style.dim(&format!("  {}", facts.join(" · "))));
    lines.push(String::new());
    let body = e.steps.first().map_or("", |s| s.body.as_str());
    lines.extend(body.lines().map(|l| if l.is_empty() { String::new() } else { format!("  {}", crate::approval::domain::rule_policy::visible(l)) }));
    lines.push(String::new());
    lines.push(style.dim(format!("  run it: /skill {} {}", e.name, if e.meta.hint.is_empty() { "" } else { &e.meta.hint }).trim_end()));
    lines.join("\n")
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
    if !hooks.untrusted_settings().is_empty() {
        lines.push(format!("{} this project's .claude/settings.json is off until you trust it: /hooks trust. It holds:", style.yellow("⚠")));
        lines.extend(hooks.untrusted_settings().iter().map(|(what, value)| format!("  {}  {}", style.cyan(&visible(what)), visible(value))));
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

/// What the project's `.claude/settings.json` makes Claude Code run or allow, which
/// `/hooks trust` approves together with the hooks (see `hooks::claude_settings_preview`).
pub fn approved_settings_text(preview: &[(String, String)], style: Style) -> String {
    if preview.is_empty() {
        return String::new();
    }
    let mut lines = vec!["  project .claude/settings.json approved:".to_string()];
    lines.extend(preview.iter().map(|(what, value)| format!("  {}  {}", style.cyan(&visible(what)), visible(value))));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::application::extension_library::Origin;
    use crate::approval::domain::extension_frontmatter::Meta;
    use std::path::PathBuf;

    #[test]
    fn rows_name_the_usage_the_origin_and_the_steps() {
        let lib = Library {
            entries: vec![Entry { name: "ship".into(), kind: Kind::Recipe, origin: Origin::User, meta: Meta { description: "Ship".into(), hint: "[tag]".into(), mode: None }, steps: vec![], path: PathBuf::new() }],
            problems: vec!["x.md: no body".into()],
        };
        let text = library_text(&lib, Style::off());
        assert!(text.contains("/ship [tag]  Ship (user · 0 steps)") && text.contains("⚠ x.md: no body"), "{text}");
        assert!(recipes_text(&Library::default(), Style::off()).contains("no recipes yet"));
        assert!(recipes_text(&lib, Style::off()).contains("/recipe <name>"));
    }

    fn skill(name: &str, description: &str, hint: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::Skill,
            origin: Origin::Project,
            meta: Meta { description: description.into(), hint: hint.into(), mode: None },
            steps: vec![crate::approval::application::extension_library::Step { title: name.into(), body: "Do it.\n\nThen $ARGUMENTS".into() }],
            path: PathBuf::from("/p/.jev/skills/x/SKILL.md"),
        }
    }

    #[test]
    fn skills_line_up_and_explain_how_to_use_them() {
        let lib = Library { entries: vec![skill("db", "Query the db", "[table]"), skill("review-pr", "", "")], problems: vec!["a/SKILL.md: no body".into(), "b.md: other".into()] };
        let text = skills_text(&lib, Style::off());
        assert!(text.starts_with("skills (2)"), "{text}");
        assert!(text.contains("  /db [table]  Query the db (project)\n"), "{text}");
        assert!(text.contains("  /review-pr   no description (project)"), "{text}");
        assert!(text.contains("⚠ a/SKILL.md: no body") && !text.contains("b.md"), "only skill problems: {text}");
        assert!(text.contains("/skill show <name>") && text.contains("/skill new <name>"), "{text}");
    }

    #[test]
    fn no_skills_says_how_to_start_one() {
        let text = skills_text(&Library::default(), Style::off());
        assert!(text.contains("no skills yet") && text.contains("/skill new <name>"), "{text}");
    }

    #[test]
    fn a_skill_shows_its_header_body_and_how_to_run_it() {
        let text = skill_text(&skill("db", "Query the db", "[table]"), Style::off());
        assert!(text.starts_with("/db [table]  (project)\n  Query the db\n  /p/.jev/skills/x/SKILL.md\n\n  Do it.\n\n  Then $ARGUMENTS\n"), "{text}");
        assert!(text.ends_with("run it: /skill db [table]"), "{text}");
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
        use crate::approval::application::extensions::Places;
        let dir = std::env::temp_dir().join(format!("jev-hooks-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".jev")).unwrap();
        std::fs::write(dir.join(".jev/hooks.json"), r#"{"hooks":[{"event":"turn_start","run":"echo first\n\u001bsecond"}]}"#).unwrap();
        let places = Places { project: dir.clone(), user: Some(dir.join("cfg")) };
        let loaded = Hooks::load(&places);
        let text = hooks_text(&loaded, Style::off());
        assert!(text.contains("turn_start") && text.contains("echo first␊␛second"), "{text}");
        let (_, defs) = crate::approval::application::extension_hooks::trust_project_with_preview(places.user.as_deref().unwrap(), &dir).unwrap();
        let approved = approved_hooks_text(&defs, Style::off());
        assert!(approved.contains("turn_start") && approved.contains("echo first␊␛second"), "{approved}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
