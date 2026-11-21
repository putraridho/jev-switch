//! Error classification.

use super::*;

    #[test]
    fn classifies_errors() {
        let b = backend();
        assert!(matches!(b.classify_error("You've hit your usage limit"), ErrorKind::Quota { .. }));
        assert_eq!(b.classify_error("unexpected status 401 Unauthorized: Missing bearer or basic authentication"), ErrorKind::Auth);
        assert_eq!(b.classify_error("Missing bearer token"), ErrorKind::Auth);
        assert_eq!(b.classify_error("weird"), ErrorKind::Other);
        let outside_git = "Not inside a trusted directory and --skip-git-repo-check was not specified.";
        assert_eq!(b.classify_error(outside_git), ErrorKind::UnusableHere);
        assert_eq!(b.classify_error("fatal: not a git repository"), ErrorKind::UnusableHere);
    }

    #[test]
    fn ordinary_text_with_401_inside_is_not_an_auth_failure() {
        let b = CodexBackend::codex();
        assert_eq!(b.classify_error("added 4012 rows to the table"), ErrorKind::Other);
        assert_eq!(b.classify_error("HTTP 401 Unauthorized"), ErrorKind::Auth);
    }
