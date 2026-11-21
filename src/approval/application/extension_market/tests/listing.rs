use super::*;
use crate::approval::application::extension_market::installer::{install, update};
use crate::approval::application::extension_market::listing::{list, State};

#[test]
fn list_notices_edits_and_update_restores_them() {
    let root = temp("update");
    let (src, user) = (root.join("src"), root.join("cfg"));
    package(&src, "1.0.0", "v1");
    install(&SystemHost, &user, &src).unwrap();
    assert_eq!(update(&SystemHost, &user, "hello-kit").unwrap(), None, "same version, intact: nothing to do");

    std::fs::write(user.join("extensions/hello-kit/commands/hello.md"), "edited").unwrap();
    assert_eq!(list(&user)[0].state, State::Changed(vec!["commands/hello.md".into()]));
    let healed = update(&SystemHost, &user, "hello-kit").unwrap().unwrap();
    assert_eq!(healed.replaced.as_deref(), Some("1.0.0"));
    assert_eq!(list(&user)[0].state, State::Verified);

    package(&src, "1.1.0", "v1.1");
    let up = update(&SystemHost, &user, "hello-kit").unwrap().unwrap();
    assert_eq!((up.version.as_str(), up.replaced.as_deref()), ("1.1.0", Some("1.0.0")));
    assert_eq!(std::fs::read_to_string(user.join("extensions/hello-kit/commands/hello.md")).unwrap(), "v1.1");
    assert!(update(&SystemHost, &user, "nope").unwrap_err().0.contains("not installed"));
}
