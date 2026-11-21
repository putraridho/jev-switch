//! `allowed_models_from_cache`.

use crate::backend::infrastructure::codex::codex_cli::models::allowed_models_from_cache;

    #[test]
    fn allowed_models_are_the_listed_ones() {
        let cache = r#"{"fetched_at":"x","models":[
            {"slug":"gpt-a","visibility":"list"},
            {"slug":"gpt-hidden","visibility":"hide"},
            {"slug":"gpt-b","visibility":"list"},
            {"visibility":"list"}]}"#;
        assert_eq!(allowed_models_from_cache(cache), ["gpt-a", "gpt-b"]);
        assert!(allowed_models_from_cache("nope").is_empty());
        assert!(allowed_models_from_cache("{}").is_empty());
    }
