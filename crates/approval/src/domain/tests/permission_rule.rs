use super::*;

#[test]
fn command_line_rules_split_outside_parentheses_only() {
    assert_eq!(super::split_rules("Bash(cargo test) Edit,Bash(git status:*)"), ["Bash(cargo test)", "Edit", "Bash(git status:*)"]);
    assert!(super::split_rules("  ").is_empty());
}

#[test]
fn command_line_rules_allow_what_they_name_and_nothing_more() {
    use super::{allowed_by, split_rules};
    fn bash(command: &str) -> Option<&str> { Some(command) }
    let rules = split_rules("Bash(python3 *) Bash(git status:*) Bash(cargo test) Edit");
    // The command from a real Antigravity run: `;` and parentheses inside quotes are text.
    assert!(allowed_by(&rules, "Bash", bash("python3 -c 'import m; print(m.add(2,3))'")));
    assert!(allowed_by(&rules, "Bash", bash("git status --short")));
    assert!(allowed_by(&rules, "Bash", bash("cargo test")));
    assert!(allowed_by(&rules, "Bash", bash("cargo test && git status")));
    assert!(allowed_by(&rules, "Edit", None));
    // Not named, or more than named.
    assert!(!allowed_by(&rules, "Bash", bash("cargo test --release")));
    assert!(!allowed_by(&rules, "Bash", bash("git statusx")));
    assert!(!allowed_by(&rules, "Bash", bash("python3 x.py && rm -rf build")));
    assert!(!allowed_by(&rules, "Bash", bash("python3 $(cat cmd)")));
    assert!(!allowed_by(&rules, "Bash", bash("python3 x.py > out.txt")));
    assert!(!allowed_by(&rules, "Write", None));
    assert!(!allowed_by(&rules, "Bash", None));
    assert!(!allowed_by(&[], "Bash", bash("ls")));
    assert!(allowed_by(&split_rules("Bash"), "Bash", None));
    assert!(allowed_by(&split_rules("Bash"), "Bash", bash("anything at all")));
}

#[test]
fn rule_syntax_is_checked_before_it_is_saved() {
    for ok in ["Bash(cargo test:*)", "Read(//tmp/**)", "WebSearch", "mcp__jev__delegate", "  Write(//a b)  "] {
        assert_eq!(validate_rule(ok).unwrap(), ok.trim());
    }
    for bad in ["", "   ", "Bash(", "Bash()", "Bash( )", "(x)", "9Bash", "Bash(a\nb)", "Ba sh(x)", "Bash(x) y"] {
        assert!(validate_rule(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_rule_has_to_be_one_claude_code_will_read() {
    // The rule `all` was written to a settings file and skipped on every start.
    for bad in ["all", "All", "any", "*", "everything", "bash", "bash(ls:*)", "read(//tmp/**)"] {
        assert!(validate_rule(bad).is_err(), "{bad:?}");
    }
    assert!(validate_rule("all").unwrap_err().contains("/mode auto"));
    assert_eq!(validate_rule("mcp__jev__delegate").unwrap(), "mcp__jev__delegate");
}
