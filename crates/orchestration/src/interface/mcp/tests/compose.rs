//! Command-line options and the auto-merge switch.

use std::path::PathBuf;

use crate::interface::mcp::compose::parse_args;
use crate::interface::mcp::lazy_delegator::auto_merge_on;

#[test]
fn auto_merge_is_on_unless_switched_off() {
    for on in [None, Some("1"), Some("on"), Some(""), Some("yes")] {
        assert!(auto_merge_on(on), "{on:?}");
    }
    for off in ["0", "off", "OFF", " false ", "no"] {
        assert!(!auto_merge_on(Some(off)), "{off}");
    }
}

#[test]
fn command_line_options() {
    let (dir, planner) = parse_args(["--dir".to_string(), "/repo".into(), "--planner".into(), "claude:claude-opus-5-5".into()]).unwrap();
    assert_eq!((dir, planner.as_str()), (Some(PathBuf::from("/repo")), "claude:claude-opus-5-5"));
    assert_eq!(parse_args([]).unwrap(), (None, String::new()));
    assert!(parse_args(["--bogus".to_string()]).is_err());
    assert!(parse_args(["--dir".to_string()]).is_err());
}
