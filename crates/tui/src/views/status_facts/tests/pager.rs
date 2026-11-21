use super::super::pager::{pager_command, should_page};

#[test]
fn the_pager_is_pager_or_less() {
    assert_eq!(pager_command(None), ("less".into(), vec!["-R".into()]));
    assert_eq!(pager_command(Some("  ")), ("less".into(), vec!["-R".into()]));
    assert_eq!(pager_command(Some("bat -p --color=always")), ("bat".into(), vec!["-p".into(), "--color=always".into()]));
    assert_eq!(pager_command(Some("cat")), ("cat".into(), vec![]));
}

#[test]
fn only_long_output_on_a_terminal_is_paged() {
    let long = "x\n".repeat(100);
    let short = "x\n".repeat(5);
    assert!(should_page(&long, Some(40), true));
    assert!(!should_page(&long, Some(40), false), "never without a terminal");
    assert!(!should_page(&short, Some(40), true));
    assert!(should_page(&"x\n".repeat(23), None, true), "an unknown height is taken as 24 rows");
    assert!(!should_page(&"x\n".repeat(22), None, true));
}
