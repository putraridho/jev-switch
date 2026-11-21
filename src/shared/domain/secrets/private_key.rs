use super::{Finding, SecretKind};

const PRIVATE_KEY_BEGIN: &str = "-----BEGIN ";
const PRIVATE_KEY_MARK: &str = "PRIVATE KEY-----";
const PRIVATE_KEY_END: &str = "-----END ";

/// `-----BEGIN ... PRIVATE KEY-----` up to its END line (or the end of the text when the
/// block is cut off: half a key is still a key).
pub(super) fn private_keys(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(PRIVATE_KEY_BEGIN) {
        let start = from + i;
        let label_from = start + PRIVATE_KEY_BEGIN.len();
        let header_end = text[label_from..].find("-----").map_or(text.len(), |e| label_from + e + 5);
        let header = &text[start..header_end];
        if !header.contains(PRIVATE_KEY_MARK) {
            from = start + PRIVATE_KEY_BEGIN.len();
            continue;
        }
        let end = match text[start..].find(PRIVATE_KEY_END) {
            Some(e) => {
                let after = start + e + PRIVATE_KEY_END.len();
                text[after..].find("-----").map_or(text.len(), |d| after + d + 5)
            }
            None => text.len(),
        };
        out.push(Finding { kind: SecretKind::PrivateKey, range: start..end });
        from = end;
    }
    out
}
