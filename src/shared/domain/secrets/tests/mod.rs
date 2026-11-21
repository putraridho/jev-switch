use super::*;

mod named;
mod redaction;
mod tokens;

fn kinds(text: &str) -> Vec<SecretKind> {
    scan(text).into_iter().map(|f| f.kind).collect()
}
