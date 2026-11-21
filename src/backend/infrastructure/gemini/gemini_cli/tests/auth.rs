//! `is_configured` and its settings parsing.

use crate::backend::infrastructure::gemini::gemini_cli::auth::{auth_looks_configured, is_configured, AUTH_ENV_VARS};

    #[test]
    fn auth_is_configured_by_env_or_a_named_auth_type() {
        assert!(auth_looks_configured(None, true));
        assert!(auth_looks_configured(Some("{}"), true));
        assert!(!auth_looks_configured(None, false));
        assert!(!auth_looks_configured(Some("{}"), false));
        assert!(auth_looks_configured(Some(r#"{"security":{"auth":{"selectedType":"oauth-personal"}}}"#), false));
        assert!(auth_looks_configured(Some(r#"{"selectedAuthType":"gemini-api-key"}"#), false));
    }

    #[test]
    fn auth_settings_edge_cases() {
        assert!(!auth_looks_configured(Some(r#"{"security":{"auth":{"selectedType":"  "}}}"#), false));
        assert!(!auth_looks_configured(Some(r#"{"security":{"auth":{}}}"#), false));
        assert!(!auth_looks_configured(Some("not json"), false));
        assert!(!auth_looks_configured(Some(""), false));
        let commented = "{\n  // how to log in\n  \"security\": {\"auth\": {\"selectedType\": \"vertex-ai\"}}\n}";
        assert!(auth_looks_configured(Some(commented), false));
    }

    #[test]
    fn is_configured_is_the_one_rule_for_settings_and_environment() {
        let commented = "{\n  // how to log in\n  \"security\": {\"auth\": {\"selectedType\": \"vertex-ai\"}}\n}";
        let none = |_: &str| false;
        assert!(is_configured(Some(commented), none), "a commented settings.json that names an auth type");
        for var in AUTH_ENV_VARS {
            assert!(is_configured(None, |n| n == var), "{var} alone is enough");
        }
        assert!(is_configured(None, |n| n == "GOOGLE_GENAI_USE_VERTEXAI"), "Vertex env only");
        assert!(!is_configured(None, none));
        assert!(!is_configured(Some("{}"), none));
        assert!(!is_configured(Some(r#"{"theme":"dark"}"#), |n| n == "SOMETHING_ELSE"));
    }
