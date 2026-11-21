use super::*;

/// What replaces a question whose tool call the CLI took back.
pub(super) const WITHDRAWN: &str = "(the tool call was withdrawn)";

/// Move up `rows` rows and clear from there to the end of the screen.
pub(super) fn erase_rows_above(rows: usize) -> String {
    format!("\x1b[{rows}A\r\x1b[J")
}

/// Show `text` in the pager when there is a terminal to page on. False when it could not.
pub(super) fn page_on_terminal(text: &str) -> bool {
    use std::io::IsTerminal;
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return false;
    }
    // The pager owns the keyboard: the type-ahead reader stays off it, and the terminal is in
    // the mode it had before the turn, which is what the pager expects to find and put back.
    let _owner = lineedit::approval::STDIN_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    crate::tui::input::typeahead::restore_turn_mode();
    crate::tui::repl::session::pager::page(text)
}

/// The text with the two columns of indent every console line has.
pub(super) fn indented(text: &str) -> String {
    format!("  {}", text.replace('\n', "\n  "))
}

impl Inner {
    /// The approval question laid out for the terminal as it is right now.
    pub(super) fn question(&self, tool: &str, review: &Review, always: bool, note: Option<&str>) -> uifmt::Prompt {
        uifmt::permission_prompt(&uifmt::Question {
            tool,
            notes: &review.notes,
            lines: &review.lines,
            always,
            note,
            style: self.cfg.err_style,
            // Two columns go to the indent every console line has.
            width: self.width().saturating_sub(2),
        })
    }
}

impl Console {
    /// The approval path of [`Ui::ask_permission`], with the pager as a parameter. Stops the
    /// status line, shows ALL of what one key approves (a long input through `pager`, or in
    /// place when there is none) and returns the [`Answer`]: it waits for the key without
    /// the lock, so the rest of the program keeps printing. Dropping it unanswered stops the
    /// wait and leaves the question on screen for [`Ui::permission_withdrawn`] to take away.
    /// `offer` is the engine's: `a` is offered only when it is `Ok`, and nothing is derived
    /// or saved here; the answer line comes from [`Ui::permission_settled`] once the engine
    /// has saved.
    ///
    /// The pager runs on a blocking thread, awaited as the first part of the answer: the
    /// engine keeps reading the CLI meanwhile, so a withdrawal or a result is seen at once.
    /// Everything printed while it is open is held and printed when it closes, and a question
    /// withdrawn meanwhile is never printed at all (only the withdrawn line is). The pager
    /// itself cannot be abandoned: it stays until the person closes it, and the turn waits
    /// for that ([`Console::pager_closed`]) before giving the terminal back.
    pub(super) fn ask_with_pager(&self, req: &PermissionRequest, offer: &Offer, pager: Pager) -> Answer {
        if let Some(front) = self.front() {
            return self.ask_front(&front, req, offer);
        }
        let Some(keys) = self.keys.clone() else {
            return Box::pin(std::future::ready(Some(Decision::Deny { reason: backend::NO_ONE_TO_ASK.to_string() })));
        };
        let offered = offer.is_ok();
        let note = match offer {
            Ok(always_plan::Always { folder: Some(dir), .. }) => Some(format!("[a] also lets Claude Code use everything in the folder {}", dir.display())),
            Ok(_) => None,
            Err(refusal) => Some(refusal.line()),
        };
        let project = self.lock().cfg.project.clone();
        let review = review::review(&req.tool, &req.input, Some(&project));
        let question = PendingQuestion { tool: req.tool.clone(), review, offered, note };

        let full = {
            let mut g = self.lock();
            g.erase();
            g.asking = Some(0);
            question.layout(&g).full
        };
        let console = self.clone();
        let Some(full) = full else {
            // Short: on screen now, before the answer is first polled.
            console.show_question(&question, false);
            return Box::pin(async move { console.wait_for_key(keys, offered).await });
        };
        Box::pin(async move {
            let turn = console.pager_turn.clone().lock_owned().await;
            console.begin_paging();
            // The guard goes with the pager, so paging ends when IT does, even if this
            // answer was dropped (withdrawn) meanwhile.
            let guard = PagingGuard { console: console.clone(), _turn: turn };
            let shown = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                pager(&full)
            })
            .await
            .unwrap_or(false);
            console.show_question(&question, shown);
            console.wait_for_key(keys, offered).await
        })
    }

    /// Print the question (the whole input in place unless `paged` showed it), and remember
    /// how many rows it takes while it is the last thing on screen.
    pub(super) fn show_question(&self, question: &PendingQuestion, paged: bool) {
        // A type-ahead read that began before the question must end before it shows, so no
        // key pressed in answer is taken as typing (it holds the lock one poll at most).
        crate::tui::input::typeahead::wait_for_reader();
        let mut g = self.lock();
        // The terminal may have been resized while the pager ran.
        let prompt = question.layout(&g);
        let text = match (&prompt.full, paged) {
            (Some(full), false) => format!("{full}\n{}", prompt.tail),
            _ => prompt.text,
        };
        let text = indented(&text);
        g.emit_err(&text);
        g.asking = Some(text.lines().count());
    }

    /// Wait for the one key that answers.
    async fn wait_for_key(&self, keys: KeyReader, offered: bool) -> Option<Decision> {
        let key = keys(offered).await;
        self.lock().asking = None;
        decision_for(key, offered)
    }

    /// The approval path with a front end: the question goes to it as a card, and the card's
    /// answer comes back on a channel. Dropping the answer (the CLI withdrew the call) closes
    /// the channel, which the front sees, and [`Ui::permission_withdrawn`] follows.
    pub(super) fn ask_front(&self, front: &Front, req: &PermissionRequest, offer: &Offer) -> Answer {
        let offered = offer.is_ok();
        let note = match offer {
            Ok(always_plan::Always { folder: Some(dir), .. }) => Some(format!("\"don't ask again\" also lets Claude Code use everything in the folder {}", dir.display())),
            Ok(_) => None,
            Err(refusal) => Some(refusal.line()),
        };
        let project = {
            let mut g = self.lock();
            g.asking = Some(0);
            g.cfg.project.clone()
        };
        let review = review::review(&req.tool, &req.input, Some(&project));
        let (reply, answer) = tokio::sync::oneshot::channel();
        front.send(FrontMsg::Approval(Box::new(ApprovalAsk {
            tool: req.tool.clone(),
            notes: review.notes,
            lines: review.lines,
            always: offer.as_ref().ok().map(always_plan::Always::label),
            note,
            reply,
        })));
        let console = self.clone();
        Box::pin(async move {
            // A front that went away without answering says no.
            let key = answer.await.unwrap_or(AnswerKey::No);
            console.lock().asking = None;
            decision_for(key, offered)
        })
    }
}

/// What a key at an approval question decides. "Always" counts as a plain yes when nothing
/// could be saved.
pub(super) fn decision_for(key: AnswerKey, offered: bool) -> Option<Decision> {
    match key {
        AnswerKey::Yes => Some(Decision::Allow),
        AnswerKey::Always if offered => Some(Decision::AllowAlways),
        AnswerKey::Always => Some(Decision::Allow),
        AnswerKey::No => Some(Decision::Deny { reason: "the user denied this tool call".to_string() }),
        AnswerKey::Interrupt => None,
    }
}

/// An approval question, owned, so it can be laid out again after the pager.
pub(super) struct PendingQuestion {
    tool: String,
    review: Review,
    offered: bool,
    note: Option<String>,
}

impl PendingQuestion {
    pub(super) fn layout(&self, g: &Inner) -> uifmt::Prompt {
        g.question(&self.tool, &self.review, self.offered, self.note.as_deref())
    }
}

/// Ends paging when dropped (the pager returned, panicked, or never got to run), then lets
/// the next pager have its turn.
pub(super) struct PagingGuard {
    console: Console,
    _turn: tokio::sync::OwnedMutexGuard<()>,
}

impl Drop for PagingGuard {
    fn drop(&mut self) {
        self.console.end_paging();
    }
}

/// `what` without a trailing period, so a detail line reads cleanly.
pub(super) fn one_sentence(what: &str) -> String {
    what.trim().trim_end_matches('.').to_string()
}
