//! What each message from the session and a paste does to the screen state.

use super::*;

impl App {
    /// A message from the session.
    pub fn on_front(&mut self, msg: FrontMsg) -> Vec<Action> {
        match msg {
            FrontMsg::Print(text) => return vec![Action::Print(text)],
            FrontMsg::TurnStarted => {
                self.live_answer = None;
                self.working = Some(Working { label: "thinking".into(), activity: None, started: Instant::now() });
            }
            FrontMsg::TurnEnded => {
                self.live_answer = None;
                self.working = None;
                self.approval = None;
                self.elicitation = None;
            }
            FrontMsg::Label(label) => {
                if let Some(w) = self.working.as_mut() {
                    w.label = label;
                    w.activity = None;
                }
            }
            FrontMsg::Activity(text) => {
                if let Some(w) = self.working.as_mut() {
                    w.activity = Some(text);
                }
            }
            FrontMsg::Approval(ask) => self.approval = Some(Approval::new(*ask, Instant::now())),
            FrontMsg::ApprovalGone => self.approval = None,
            FrontMsg::Elicitation { request, reply } => self.elicitation = Some(Elicitation::new(request, reply, Instant::now())),
            FrontMsg::ElicitationGone => self.elicitation = None,
            FrontMsg::Confirm { question, reply } => self.confirm = Some(Confirm { question, reply: Some(reply), selected: 1 }),
            FrontMsg::Suspend { ack } => {
                self.suspended = true;
                return vec![Action::Suspend(ack)];
            }
            FrontMsg::Resume => {
                self.suspended = false;
                return vec![Action::Resume];
            }
            FrontMsg::ClearScreen => {
                self.live_answer = None;
                return vec![Action::ClearScreen];
            }
            FrontMsg::Bell => return vec![Action::Bell],
            FrontMsg::AnswerDelta(text) => match self.live_answer.as_mut() {
                Some(live) => live.push_str(&text),
                None => self.live_answer = Some(text),
            },
            FrontMsg::AnswerReset => self.live_answer = None,
            FrontMsg::Steered(text) => {
                if let Some(at) = self.queue.iter().position(|q| *q == text) {
                    self.queue.remove(at);
                }
            }
        }
        Vec::new()
    }

    /// Pasted text goes into the box as one piece (line breaks kept).
    pub fn on_paste(&mut self, text: &str) {
        if let Some(e) = self.elicitation.as_mut() {
            if e.editing() { e.editor.insert_str(text); }
        } else if self.approval.is_none() && self.confirm.is_none() {
            self.editor.insert_str(text);
            self.edited();
        }
    }
}
