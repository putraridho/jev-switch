//! Deliver server questions without involving tool permission rules.

use super::*;
use backend::{ElicitationRequest, ElicitationAnswer, ElicitationKind};
use jev_turn::application::ports::ElicitationReply;
use jev_approval::domain::visible_text::visible;

impl Console {
    pub(super) fn ask_elicitation(&self, req: &ElicitationRequest) -> ElicitationReply {
        if let Some(front) = self.front() {
            let (reply, answer) = tokio::sync::oneshot::channel();
            self.lock().asking = Some(0);
            front.send(FrontMsg::Elicitation { request: req.clone(), reply });
            let console = self.clone();
            return Box::pin(async move {
                let answer = answer.await.unwrap_or(ElicitationAnswer::Cancel);
                console.lock().asking = None;
                answer
            });
        }
        let Some(keys) = self.keys.clone() else {
            return Box::pin(std::future::ready(ElicitationAnswer::Cancel));
        };
        let simple = matches!(&req.kind, ElicitationKind::Form(fields) if fields.is_empty()) || matches!(&req.kind, ElicitationKind::Url(_));
        if !simple {
            self.err("MCP form cancelled: use the default interactive UI (unset JEV_CLASSIC_UI) to answer forms.");
            return Box::pin(std::future::ready(ElicitationAnswer::Cancel));
        }
        let mut text = format!("{} needs your input\n{}", visible(&req.server), visible(&req.message));
        if let ElicitationKind::Url(url) = &req.kind { text.push_str(&format!("\nOpen this URL in your browser: {}", visible(url))); }
        text.push_str("\n[y] send answer  [n] decline  [Esc] decline  [Ctrl+C] cancel");
        {
            let mut g = self.lock();
            g.erase();
            g.asking = Some(0);
            g.emit_err(&indented(&text));
        }
        let console = self.clone();
        Box::pin(async move {
            let key = keys(false).await;
            console.lock().asking = None;
            match key {
                AnswerKey::Yes => ElicitationAnswer::Accept(Vec::new()),
                AnswerKey::No => ElicitationAnswer::Decline,
                _ => ElicitationAnswer::Cancel,
            }
        })
    }
}
