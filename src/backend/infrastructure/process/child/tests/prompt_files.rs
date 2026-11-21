use super::*;
use std::sync::Mutex;
use tokio::process::Command;

#[tokio::test]
async fn run_turn_cleans_up_the_backends_files_after_every_kind_of_turn() {
    struct Writes {
        files: PromptFiles,
        path: Mutex<Option<PathBuf>>,
        script: &'static str,
    }
    impl Backend for Writes {
        fn provider(&self) -> ProviderId {
            ProviderId::Grok
        }
        fn command(&self, req: &TurnRequest) -> Command {
            *self.path.lock().unwrap() = self.files.arg(&req.prompt).file().map(Path::to_path_buf);
            let mut c = Command::new("sh");
            c.arg("-c").arg(self.script).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            match line.trim_end().split_once(':') {
                Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
        fn cleanup(&self) {
            self.files.cleanup();
        }
    }
    let dir = scratch_dir("runner");
    let mut request = req();
    request.prompt = "p".repeat(MAX_PROMPT_ARG_BYTES + 1);
    for script in ["echo FIN:ok", "exit 1"] {
        let b = Writes { files: PromptFiles::in_dir(dir.clone()), path: Mutex::new(None), script };
        let r = run_turn(&b, &request, Duration::from_secs(5), &mut |_| {}).await;
        assert_eq!(r.is_ok(), script.contains("FIN"));
        let path = b.path.lock().unwrap().clone().expect("the long prompt was written");
        assert!(!path.exists(), "{script}: left {path:?} behind");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
