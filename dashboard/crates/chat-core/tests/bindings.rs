//! Exports TypeScript bindings for `jev-chat-core` into `dashboard/src/lib/ipc/chat-bindings/`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

#[cfg(feature = "bindings")]
mod bindings {
    use jev_chat_core::ipc::{
        ChatAttachResult, ChatAttachment, ChatAttachmentKind, ChatCatalogModel, ChatDecision,
        ChatDirListing, ChatEdit, ChatEvent, ChatMessage, ChatMode, ChatPermissionPrompt,
        ChatPermissionReply, ChatPin, ChatProviderStatus, ChatRole, ChatRouteInfo,
        ChatSessionState, ChatToolCall, ChatTurnStatus, ChatUsage,
    };
    use std::path::PathBuf;
    use ts_rs::TS;

    fn bindings_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/lib/ipc/chat-bindings")
    }

    fn export_one<T: TS + 'static>(out: &mut Vec<(String, String)>) {
        let name = format!("{}.ts", T::name());
        let mut text = T::export_to_string().unwrap().replace(
            "import type { ProviderKey } from \"./ProviderKey\";",
            "import type { ProviderKey } from \"../bindings/ProviderKey\";",
        );
        while let Some(start) = text.find("Array<") {
            let after = &text[start + 6..];
            let Some(end) = after.find('>') else { break };
            let inner = &after[..end];
            let replaced = format!("{inner}[]");
            text = format!("{}{}{}", &text[..start], replaced, &after[end + 1..]);
        }
        for banned in [
            "any",
            "unknown",
            "never",
            "bigint",
            "object",
            "Function",
            "undefined",
        ] {
            assert!(
                !text.contains(&format!(": {banned}")),
                "{name} contains banned TS type `{banned}`:\n{text}"
            );
        }
        out.push((name, text));
    }

    #[test]
    fn export_chat_bindings() {
        let mut files = Vec::new();
        export_one::<ChatMode>(&mut files);
        export_one::<ChatPin>(&mut files);
        export_one::<ChatRole>(&mut files);
        export_one::<ChatTurnStatus>(&mut files);
        export_one::<ChatDecision>(&mut files);
        export_one::<ChatEdit>(&mut files);
        export_one::<ChatToolCall>(&mut files);
        export_one::<ChatRouteInfo>(&mut files);
        export_one::<ChatUsage>(&mut files);
        export_one::<ChatAttachmentKind>(&mut files);
        export_one::<ChatAttachment>(&mut files);
        export_one::<ChatAttachResult>(&mut files);
        export_one::<ChatMessage>(&mut files);
        export_one::<ChatPermissionPrompt>(&mut files);
        export_one::<ChatPermissionReply>(&mut files);
        export_one::<ChatProviderStatus>(&mut files);
        export_one::<ChatCatalogModel>(&mut files);
        export_one::<ChatDirListing>(&mut files);
        export_one::<ChatSessionState>(&mut files);
        export_one::<ChatEvent>(&mut files);

        let dir = bindings_dir();
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in &files {
            std::fs::write(dir.join(name), text).unwrap();
        }
    }
}
