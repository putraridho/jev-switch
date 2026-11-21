//! Error classification.

use super::*;

    #[test]
    fn gemini_exit_41_stderr_is_auth() {
        assert_eq!(gemini().classify_error(GEMINI_AUTH_STDERR.trim()), ErrorKind::Auth);
    }

    #[test]
    fn untrusted_workspace_is_unusable_here() {
        let msg = "Gemini CLI is not running in a trusted directory. To proceed, either use `--skip-trust`, set the \
                   `GEMINI_CLI_TRUST_WORKSPACE=true` environment variable, or trust this directory in interactive mode.";
        assert_eq!(gemini().classify_error(msg), ErrorKind::UnusableHere);
        assert_eq!(gemini().classify_error("The workspace is not trusted"), ErrorKind::UnusableHere);
        assert_eq!(agy().classify_error("folder is not trusted"), ErrorKind::UnusableHere);
    }

    #[test]
    fn classifies_other_errors_and_common_ones() {
        assert_eq!(gemini().classify_error("segmentation fault"), ErrorKind::Other);
        let retired = "An unexpected critical error occurred: This client is no longer supported for Gemini Code Assist for individuals. To continue using Gemini, please migrate to the Antigravity suite of products: https://antigravity.google";
        assert_eq!(gemini().classify_error(retired), ErrorKind::Unsupported);
        assert_eq!(agy().classify_error(retired), ErrorKind::Unsupported);
        assert!(matches!(gemini().classify_error("You have hit your usage limit"), ErrorKind::Quota { .. }));
        assert_eq!(gemini().classify_error("RESOURCE_EXHAUSTED"), ErrorKind::Quota { resets_at: None });
        assert_eq!(reset_after("Your quota will reset after 59s"), Some("59s".into()));
        assert_eq!(reset_after("no hint here"), None);
    }

    #[test]
    fn auth_wording_must_say_the_login_failed_not_just_mention_authentication() {
        let b = GeminiBackend::gemini_cli();
        assert_eq!(b.classify_error("authenticate the user in the request handler"), ErrorKind::Other);
        assert_eq!(b.classify_error("the sign in page redirects twice"), ErrorKind::Other);
        assert_eq!(b.classify_error("Authentication failed: please sign in again"), ErrorKind::Auth);
    }
