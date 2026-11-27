//! Integration tests for `jev-chat-io` service & folder browsing.
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

use jev_chat_core::ipc::{ChatMode, ChatPin};
use jev_chat_io::service::ChatServiceHandle;
use jev_usage_core::provider::ProviderKey;
use tokio::runtime::Runtime;

#[test]
fn chat_service_updates_mode_pin_workspace_and_lists_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let sub = tmp.path().join("project-a");
    std::fs::create_dir(&sub).unwrap();
    std::fs::create_dir(sub.join("src")).unwrap();
    std::fs::create_dir(sub.join(".hidden")).unwrap();

    let rt = Runtime::new().unwrap();
    let handle = ChatServiceHandle::start(tmp.path().to_path_buf(), None, rt.handle().clone());

    let listing = handle
        .list_dirs(Some(sub.to_string_lossy().into_owned()))
        .unwrap();
    assert_eq!(listing.dirs.len(), 1);
    assert!(listing.dirs[0].ends_with("/src"));

    rt.block_on(async {
        let st = handle.set_mode(ChatMode::Plan).await.unwrap();
        assert_eq!(st.mode, ChatMode::Plan);

        let st = handle
            .set_pin(ChatPin::Provider {
                provider: ProviderKey::Codex,
            })
            .await
            .unwrap();
        assert_eq!(
            st.pin,
            ChatPin::Provider {
                provider: ProviderKey::Codex
            }
        );

        let st = handle.set_orchestrate(true).await.unwrap();
        assert!(st.orchestrate);

        let st = handle
            .set_workspace(sub.to_string_lossy().into_owned())
            .await
            .unwrap();
        assert_eq!(
            st.workspace,
            std::fs::canonicalize(&sub).unwrap().to_string_lossy()
        );
    });
}

#[test]
fn default_workspace_prefers_the_repository_root_and_never_root() {
    use jev_chat_io::service::default_workspace;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = home.join("repo");
    let sub = repo.join("a/b");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir(repo.join(".git")).unwrap();
    let plain = home.join("plain");
    std::fs::create_dir(&plain).unwrap();

    assert_eq!(default_workspace(Some(sub), Some(home.clone())), repo);
    assert_eq!(
        default_workspace(Some(plain.clone()), Some(home.clone())),
        plain
    );
    assert_eq!(
        default_workspace(Some("/".into()), Some(home.clone())),
        home
    );
    assert_eq!(default_workspace(None, Some(home.clone())), home);
    assert_eq!(default_workspace(None, None), std::path::PathBuf::from("/"));
    // A home folder kept in git (dotfiles) is not taken for the project.
    std::fs::create_dir(home.join(".git")).unwrap();
    assert_eq!(
        default_workspace(Some(plain.clone()), Some(home.clone())),
        plain
    );
}
