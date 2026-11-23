//! The saved readiness records: `provider-readiness.json`, shared by every `jev-code` and
//! `delegate_mcp` process, written atomically and merged with what is already there. Only
//! labels and stamps are saved (see `ReadinessKey`), never a credential or a prompt.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::readiness::{Evidence, Failed, Readiness, ReadinessKey, Record, Rung};
use jev_shared::ProviderId;

#[derive(Serialize, Deserialize, Default)]
struct KeyWire {
    #[serde(default)]
    cli: String,
    #[serde(default)]
    backend: String,
    #[serde(default)]
    account: String,
}

#[derive(Serialize, Deserialize)]
struct EvidenceWire {
    rung: String,
    #[serde(default)]
    what: String,
    #[serde(default)]
    at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct FailedWire {
    rung: String,
    #[serde(default)]
    what: String,
    #[serde(default)]
    at_ms: u64,
    #[serde(default)]
    until_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct RecordWire {
    provider: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    key: KeyWire,
    #[serde(default)]
    evidence: Vec<EvidenceWire>,
    #[serde(default)]
    failed: Option<FailedWire>,
    #[serde(default)]
    refused: Vec<String>,
    #[serde(default)]
    updated_ms: u64,
}

fn to_wire(((provider, project), r): (&(ProviderId, String), &Record)) -> RecordWire {
    RecordWire {
        provider: provider.as_str().to_string(),
        project: project.clone(),
        key: KeyWire { cli: r.key.cli.clone(), backend: r.key.backend.clone(), account: r.key.account.clone() },
        evidence: r.evidence.iter().map(|(rung, e)| EvidenceWire { rung: rung.id().to_string(), what: e.what.clone(), at_ms: e.at_ms }).collect(),
        failed: r.failed.as_ref().map(|f| FailedWire { rung: f.rung.id().to_string(), what: f.what.clone(), at_ms: f.at_ms, until_ms: f.until_ms }),
        refused: r.refused.clone(),
        updated_ms: r.updated_ms,
    }
}

/// A record that names an unknown provider or rung is dropped, so a newer file never breaks an older build.
fn from_wire(w: RecordWire) -> Option<((ProviderId, String), Record)> {
    let evidence = w.evidence.into_iter().filter_map(|e| Some((Rung::from_id(&e.rung)?, Evidence { what: e.what, at_ms: e.at_ms }))).collect();
    let failed = w.failed.and_then(|f| Some(Failed { rung: Rung::from_id(&f.rung)?, what: f.what, at_ms: f.at_ms, until_ms: f.until_ms }));
    let key = ReadinessKey { cli: w.key.cli, backend: w.key.backend, account: w.key.account };
    Some(((w.provider.parse::<ProviderId>().ok()?, w.project), Record { key, evidence, failed, refused: w.refused, updated_ms: w.updated_ms }))
}

/// What is saved; a missing, unreadable or garbled file counts as nothing saved.
pub fn load(path: &Path) -> Readiness {
    let Some(text) = std::fs::read_to_string(path).ok() else { return Readiness::default() };
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else { return Readiness::default() };
    raw.into_iter()
        .filter_map(|v| serde_json::from_value::<RecordWire>(v).ok())
        .filter_map(from_wire)
        .collect::<BTreeMap<_, _>>()
        .into()
}

/// Merge `readiness` with what is saved (the record that changed last wins), write the
/// result, and return it.
pub fn save(path: &Path, readiness: &Readiness) -> std::io::Result<Readiness> {
    let merged = readiness.merged(&load(path));
    let wire: Vec<RecordWire> = merged.entries().iter().map(to_wire).collect();
    super::atomic_file::write_atomic(path, &serde_json::to_string(&wire)?)?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::readiness::classify;
    use jev_backend::ErrorKind;

    fn dir(name: &str) -> std::path::PathBuf {
        jev_testkit::scratch_dir::scratch_dir("readiness", name)
    }

    fn sample() -> Readiness {
        let mut r = Readiness::default();
        let key = ReadinessKey { cli: "agy:1".into(), backend: "Google login".into(), account: "t:5".into() };
        r.verify(ProviderId::Gemini, "/p#trusted", &key, Rung::Installed, "found", 100);
        r.fail(ProviderId::Gemini, "/p#trusted", &key, classify(&ErrorKind::Unsupported).unwrap(), "retired client", 200);
        r.refuse_model(ProviderId::Codex, "/p", &key, "gpt-x", 300);
        r
    }

    #[test]
    fn it_round_trips_and_a_missing_or_garbled_file_is_nothing() {
        let path = dir("round").join("provider-readiness.json");
        assert_eq!(load(&path), Readiness::default());
        let r = sample();
        assert_eq!(save(&path, &r).unwrap(), r);
        assert_eq!(load(&path), r);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path), Readiness::default());
    }

    #[test]
    fn old_and_newer_files_still_read() {
        let path = dir("compat").join("provider-readiness.json");
        // Only the required fields (as an older build wrote them), one unknown provider, one unknown rung, an extra field.
        std::fs::write(
            &path,
            r#"[{"provider":"gemini","project":"p"},
                {"provider":"nobody","project":"p"},
                {"provider":"codex","project":"p","future":1,"evidence":[{"rung":"hologram"},{"rung":"installed","what":"found","at_ms":5}],"failed":{"rung":"nope"}}]"#,
        )
        .unwrap();
        let r = load(&path);
        assert_eq!(r.entries().len(), 2);
        let codex = &r.entries()[&(ProviderId::Codex, "p".to_string())];
        assert_eq!((codex.evidence.len(), codex.failed.is_none()), (1, true));
    }

    #[test]
    fn saving_keeps_what_another_process_saved() {
        let path = dir("merge").join("provider-readiness.json");
        save(&path, &sample()).unwrap();
        let mut other = Readiness::default();
        other.refuse_model(ProviderId::Claude, "/q", &ReadinessKey::default(), "m", 400);
        assert_eq!(save(&path, &other).unwrap().entries().len(), 3);
    }
}
