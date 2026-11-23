use std::collections::BTreeSet;
use std::path::Path;

use jev_router::backend::infrastructure::gemini::gemini_cli::auth::AUTH_ENV_VARS;
use jev_router::backend::infrastructure::grok::grok_cli::API_KEY_ENV;

const EXAMPLE: &str = include_str!("../.jev-router.env.example");

fn example_keys() -> BTreeSet<&'static str> {
    EXAMPLE.lines().filter_map(|line| {
        let line = line.trim().trim_start_matches('#').trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (key, _) = line.split_once('=')?;
        let key = key.trim();
        (!key.is_empty() && key.chars().all(|character| character.is_ascii_uppercase() || character == '_' || character.is_ascii_digit())).then_some(key)
    }).collect()
}

fn public_file_keys(directory: &Path, keys: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if name == "tests" || name == "testkit" || name == "testing.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        if path.is_dir() {
            public_file_keys(&path, keys);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            let production = source.split("#[cfg(test)]").next().unwrap();
            for marker in ["lookup(", "lookup_f64(", "host.env(", "SystemHost.env(", "path_from_env(", "bin_from_env("] {
                for tail in production.split(marker).skip(1) {
                    let Some(quoted) = tail.trim_start().strip_prefix('"') else { continue };
                    let key = quoted.split('"').next().unwrap();
                    let public = key.starts_with("JEV_") || key.starts_with("CODE_") || key.ends_with("_BIN")
                        || key == "DEEPSEEK_API_KEY" || key == "DOWNGRADE_AFTER"
                        || matches!(key, "PAGER" | "EDITOR" | "VISUAL" | "CODEX_HOME");
                    if public {
                        keys.insert(key.to_string());
                    }
                }
            }
        }
    }
}

#[test]
fn example_covers_public_file_backed_configuration_readers() {
    let mut required = BTreeSet::new();
    public_file_keys(&Path::new(env!("CARGO_MANIFEST_DIR")).join("crates"), &mut required);
    assert!(!required.is_empty(), "configuration reader scan found no keys");
    let documented = example_keys();
    let missing: Vec<_> = required.iter().filter(|key| !documented.contains(key.as_str())).collect();
    assert!(missing.is_empty(), "file-backed settings missing from .jev-router.env.example: {missing:?}");
}

#[test]
fn example_covers_provider_credentials_and_jev_aliases() {
    let documented = example_keys();
    for key in AUTH_ENV_VARS.into_iter().chain([API_KEY_ENV, "DEEPSEEK_API_KEY"]) {
        assert!(documented.contains(key), "missing {key}");
    }
    let transport = include_str!("../crates/judgment/src/infrastructure/typesafe_http.rs");
    let declaration = transport.split("const KEY_NAMES:").nth(1).unwrap();
    let aliases = declaration.split_once('=').unwrap().1.split(';').next().unwrap();
    let names: Vec<_> = aliases.split('"').skip(1).step_by(2).collect();
    assert!(!names.is_empty(), "Jev credential alias declaration contained no names");
    for key in names {
        assert!(documented.contains(key), "missing Jev credential alias {key}");
    }
}

#[test]
fn copying_the_example_does_not_enable_optional_credentials_or_settings() {
    let active: Vec<_> = EXAMPLE.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).collect();
    assert_eq!(active, ["JEV_API_KEY="]);
    assert_eq!(jev_router::shared::infrastructure::env_file::parse(EXAMPLE, "JEV_API_KEY"), None);
}
