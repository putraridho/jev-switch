//! The interactive loop: keys, the session's messages and a timer in, the screen out.
//!
//! The session ([`Repl`]) runs each line on a task of its own, so the screen keeps reading
//! keys while a turn works: typing queues the next message, Esc and Ctrl-C stop the turn,
//! and approvals are answered on a card. Between lines the session comes back here.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::screen_state::{Action, App};
use crate::input::history::History;
use crate::tty::frontend_screen::Screen;
use crate::tty::title::{title_sequence, Activity};
use crate::views::frontend_view as view;
use crate::console::terminal_ui::FRAME_TICK;

/// Rows one wheel notch asks the chat to move.
const WHEEL_LINES: isize = 4;
use crate::console::front::FrontMsg;
use crate::input::line_editor as lineedit;
use crate::repl::session::{Flow, Repl};
use crate::render::Style;

mod actions;
mod status;
mod tty_io;
mod turn;
#[cfg(test)]
mod tests;

use actions::carry_out;
use status::{provider_chips, refresh, route_label, worker_chips};
use tty_io::{draw, fullscreen_wanted, keys_off, keys_on, term_env, width, KeyModes, FULLSCREEN};
use turn::{start, Done, Stops};

/// What the loop needs besides the session.
pub struct Options {
    pub style: Style,
    /// `<state dir>/history`, when there is a state folder.
    pub history: Option<PathBuf>,
}

/// The next key or paste, or `None` while the terminal belongs to a command.
async fn next_event(events: &mut Option<EventStream>) -> Option<std::io::Result<Event>> {
    match events {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// The running line, or never.
async fn finished(task: &mut Option<JoinHandle<Done>>) -> Option<Done> {
    match task {
        Some(handle) => {
            let done = handle.await.ok();
            *task = None;
            done
        }
        None => std::future::pending().await,
    }
}

/// Apply already-reported queue changes before a key can take a message back for editing.
fn key_after_updates(app: &mut App, rx: &mut mpsc::UnboundedReceiver<FrontMsg>, key: crossterm::event::KeyEvent, now: Instant) -> Vec<Action> {
    let mut actions = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        actions.extend(app.on_front(msg));
    }
    actions.extend(app.on_key(key, now));
    actions
}

/// Run the interactive session until the person quits. Returns the exit code.
pub async fn run(repl: Repl, mut rx: mpsc::UnboundedReceiver<FrontMsg>, opts: Options) -> i32 {
    let style = opts.style;
    FULLSCREEN.store(fullscreen_wanted(), std::sync::atomic::Ordering::SeqCst);
    let _modes = match keys_on() {
        Ok(()) => KeyModes,
        Err(e) => {
            repl.console.err(&format!("jev-code: cannot read keys from this terminal: {e}"));
            return 1;
        }
    };
    // The session moves into the task that runs a turn; the console is shared, so messages
    // typed meanwhile can still reach that turn.
    let console = repl.console.clone();
    let project = repl.engine.dir().to_path_buf();
    let state_dir = repl.state_dir().map(std::path::Path::to_path_buf);
    let mut workers_read = Instant::now();
    let tracked = lineedit::completion::tracked_files(&project);
    let mut app = App::new(project.clone(), History::load(opts.history.as_deref()), repl.engine.mode(), route_label(&repl), tracked);
    app.fullscreen = FULLSCREEN.load(std::sync::atomic::Ordering::SeqCst);
    refresh(&mut app, &repl);
    let (keymap, problems) = crate::input::keymap::load();
    app.keymap = keymap;
    if let Some(problem) = problems.first() {
        app.notice = Some((problem.clone(), Instant::now()));
    }

    let mut screen = Screen::new(std::io::stdout(), style.enabled);
    let mut events = Some(EventStream::new());
    let (stop_tx, stop_rx) = mpsc::unbounded_channel();
    let mut idle: Option<(Repl, Stops)> = Some((repl, Stops(stop_rx)));
    let mut task: Option<JoinHandle<Done>> = None;
    let mut mode_picked_during_turn = false;
    let mut tick = tokio::time::interval(term_env().tick(FRAME_TICK));
    let mut last_width = width();
    // The answer the suggestion was last made from: a line that brings no new answer (a
    // command) makes none. A resumed session's last answer is not suggested from.
    let mut answer_seen: Option<String> = idle.as_ref().and_then(|(r, _)| r.engine.last_answer()).map(String::from);

    // The tab title follows what the agent is doing, so a background tab says when it needs you.
    let mut title: Option<String> = None;
    let mut was_working = false;
    let code = loop {
        // Idle: catch up with the session, then run the next queued message, if any.
        if let Some((repl, _)) = idle.as_mut() {
            repl.engine.runtime.absorb_late();
            repl.console.flush_notices();
            // A provider that just became ready (or came off a cooldown) shows on the router row.
            app.providers = provider_chips(repl);
            if app.orchestrate {
                app.workers = worker_chips(state_dir.as_deref(), &project);
            }
            if app.mode != repl.engine.mode() {
                app.mode = repl.adopt_mode(app.mode);
            }
        }
        if task.is_none() && !app.suspended {
            if let Some(line) = app.queue.pop_front() {
                if let Some(started) = start(&mut idle, &line, style, &mut screen, &mut app) {
                    task = Some(started);
                }
            }
        }
        if !app.suspended {
            let activity = if app.approval.is_some() || app.confirm.is_some() {
                Activity::Waiting
            } else if app.working.is_some() {
                Activity::Running
            } else if was_working {
                Activity::Done
            } else {
                Activity::Idle
            };
            was_working = app.working.is_some() || (was_working && app.approval.is_some());
            let name = app.session_name.clone().unwrap_or_default();
            let sequence = title_sequence(activity, &name);
            if title.as_deref() != Some(sequence.as_str()) {
                screen.raw(&sequence);
                title = Some(sequence);
            }
            draw(&mut screen, &mut app, style);
        }

        let mut actions = tokio::select! {
            ev = next_event(&mut events) => match ev {
                Some(Ok(Event::Key(k))) if k.code == KeyCode::Char('v')
                    && k.modifiers == KeyModifiers::CONTROL
                    && k.kind != KeyEventKind::Release
                    && app.approval.is_none() && app.confirm.is_none()
                    && app.keymap.action(&k).is_none() => {
                    match crate::app::host::clipboard::paste_from_clipboard(state_dir.as_deref()).await {
                        Ok(text) => app.on_paste(&text),
                        Err(error) => app.notice = Some((error, Instant::now())),
                    }
                    Vec::new()
                }
                Some(Ok(Event::Key(k))) => key_after_updates(&mut app, &mut rx, k, Instant::now()),
                Some(Ok(Event::Paste(text))) => {
                    let pasted = crate::tty::terminal_environment::sanitize_paste(&text);
                    app.on_paste(&pasted.text);
                    if pasted.truncated {
                        app.notice = Some(("the paste was too long: only the first 256 KiB was kept".to_string(), Instant::now()));
                    }
                    Vec::new()
                }
                // A resize to 0 columns (a window being dragged shut, some multiplexers
                // while detaching) says nothing about the real width.
                Some(Ok(Event::Resize(0, _))) => Vec::new(),
                Some(Ok(Event::Resize(w, _))) => {
                    let w = usize::from(w);
                    if app.fullscreen {
                        screen.forget();
                    } else if w < last_width {
                        screen.resized(w);
                    }
                    last_width = w;
                    Vec::new()
                }
                Some(Ok(Event::Mouse(m))) => {
                    match m.kind {
                        MouseEventKind::ScrollUp => app.scroll_chat(WHEEL_LINES),
                        MouseEventKind::ScrollDown => app.scroll_chat(-WHEEL_LINES),
                        _ => {}
                    }
                    Vec::new()
                }
                Some(Ok(_)) => Vec::new(),
                // The terminal went away: end the session.
                Some(Err(_)) | None => vec![Action::Quit],
            },
            msg = rx.recv() => match msg {
                Some(m) => app.on_front(m),
                None => Vec::new(),
            },
            _ = tick.tick(), if app.needs_ticks() => {
                app.expire(Instant::now());
                app.step_scroll();
                // Workers start and finish while a turn runs, in other processes: look at the
                // registry about once a second.
                if workers_read.elapsed() >= Duration::from_secs(1) {
                    workers_read = Instant::now();
                    app.workers = worker_chips(state_dir.as_deref(), &project);
                }
                Vec::new()
            }
            done = finished(&mut task) => {
                match done {
                    Some((repl, stops, flow)) => {
                        let flow_code = match flow {
                            Flow::Continue => None,
                            Flow::Quit => Some(0),
                            Flow::Aborted => Some(130),
                        };
                        if mode_picked_during_turn {
                            mode_picked_during_turn = false;
                        } else {
                            app.mode = repl.engine.mode();
                        }
                        refresh(&mut app, &repl);
                        let answer = repl.engine.last_answer();
                        if answer != answer_seen.as_deref() {
                            app.suggestion = answer.and_then(crate::input::suggest::next_prompt);
                            answer_seen = answer.map(String::from);
                        }
                        idle = Some((repl, stops));
                        if let Some(c) = flow_code {
                            break c;
                        }
                    }
                    // The line's task panicked: the session is lost with it.
                    None => break 1,
                }
                Vec::new()
            }
        };
        // Everything else the session said meanwhile, in order, before the next redraw.
        while let Ok(msg) = rx.try_recv() {
            actions.extend(app.on_front(msg));
        }
        let next = carry_out(actions, &mut app, &mut screen, &mut events, &stop_tx, &mut idle, &mut mode_picked_during_turn, style, &console);
        if let Some(started) = next.start {
            task = Some(started);
        }
        if let Some(c) = next.exit {
            break c;
        }
    };

    // Let the session print what it still has (a save error), then hand the terminal back.
    let mut late = Vec::new();
    while let Ok(FrontMsg::Print(text)) = rx.try_recv() {
        late.push(text);
    }
    if app.fullscreen {
        // The alternate screen is about to go: what is left to say goes on the real one.
        drop(events);
        keys_off();
        for text in late {
            println!("{text}");
        }
    } else {
        for text in late {
            screen.print_above(&text, &view::Frame { rows: Vec::new(), cursor: None });
        }
        screen.erase();
        drop(events);
        keys_off();
    }
    if let Some((mut repl, _)) = idle {
        if let Err(e) = repl.engine.save_now() {
            eprintln!("  could not save the session: {e}");
        }
    }
    code
}
