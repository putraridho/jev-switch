use super::*;

#[test]
fn punctuation_does_not_hide_paths() {
    let p = policy("block .env from all\nblock *.pem from all\nblock data? from all");
    for prompt in ["what's in .env?", ".env!", ".env,", ".env;", ".env:", "(.env)", "\".env\"", "'.env'", "`.env`", ".env.", "a.pem:", "@(.env)", ".env:42?"] {
        let verdict = check(&p, prompt);
        assert!(crate::shared::ALL_PROVIDERS.iter().all(|provider| !verdict.permits(*provider)), "{prompt}");
    }
    assert!(!check(&p, "data?").permits(ProviderId::Claude));
    let raw = policy("block data! from all");
    assert!(!check(&raw, "data!").permits(ProviderId::Claude));
    assert!(check(&p, "environment").permits(ProviderId::Claude));
}

#[test]
fn location_suffixes_do_not_hide_paths() {
    let p = policy("block .env from all\nblock *.pem from all\nblock src/*.rs from all");
    for (prompt, path) in [
        (".env:3:1", ".env"),
        ("server.pem:12:5", "server.pem"),
        ("src/a.rs:10-20", "src/a.rs"),
        ("config/.env#L4", "config/.env"),
        ("x.pem#L1-L9", "x.pem"),
        ("@\"config/.env:4:2\",", "config/.env"),
        ("x.pem:1:2:3", "x.pem"),
        ("`config/.env`:12", "config/.env"),
        ("\".env\":3", ".env"),
        ("(`server.pem`:1:2).", "server.pem"),
    ] {
        assert!(mentioned_paths(Path::new(ROOT), prompt, None).contains(&path.to_string()), "{prompt}");
        let verdict = check(&p, prompt);
        assert!(crate::shared::ALL_PROVIDERS.iter().all(|provider| !verdict.permits(*provider)), "{prompt}");
    }
}

#[test]
fn globs_ignore_ascii_case_for_prompts_and_attachments() {
    let p = policy("block secrets/** from all\nblock *.pem from all");
    assert!(!check(&p, "Secrets/db.yml").permits(ProviderId::Claude));
    assert!(!check(&p, "/WORK/APP/Secrets/db.yml").permits(ProviderId::Claude));
    assert!(!check(&p, "deploy/KEY.PEM").permits(ProviderId::Claude));
    assert!(!p.check(Path::new(ROOT), "inspect", &[PathBuf::from("Secrets/db.yml")], None).permits(ProviderId::Claude));
    assert!(glob_matches("SECRETS/**", "secrets/db.yml"));
    assert!(!glob_matches("secrets/**", "secrets-old/db.yml"));
}
