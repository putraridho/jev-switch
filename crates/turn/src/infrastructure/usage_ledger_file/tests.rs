use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;
use crate::application::usage_account::UsageAccount;
use crate::domain::budget::{Budget, Cap, PriceTable};
use crate::infrastructure::memory_usage_ledger::KnownProcesses;
use jev_providers::domain::funding::Fundings;

use crate::domain::admission::Estimate;

fn scratch(name: &str) -> PathBuf {
    jev_testkit::scratch_dir::scratch_dir("usage-ledger", name).join(FILE)
}

#[test]
fn a_book_survives_the_file_and_reads_back_without_changing_it() {
    let path = scratch("round");
    let ledger = FileUsageLedger;
    assert_eq!(ledger.read(&path), Some(Book::default()), "no file is an empty ledger");
    let mut written = Book::default();
    assert!(ledger.transact(&path, &mut |book| {
        book.begin("s1", "/p", 10);
        book.hold("s1", Held { id: "h".into(), owner_pid: 3, owner_start: Some(9), ts_ms: 11, provider: ProviderId::DeepSeek, model: "m".into(), paid: true, tokens: 5, usd: Some(0.5) });
        book.settle("s1", None, Spent::unknown(12, "w", ProviderId::Claude, "c"));
        book.settle("s1", None, Spent { ts_ms: 13, writer: "w".into(), provider: ProviderId::Codex, model: "x".into(), usage: Usage { input_tokens: 4, cached_input_tokens: 1, output_tokens: 2, ..Usage::default() } });
        written = book.clone();
    }));
    assert_eq!(ledger.read(&path), Some(written));
}

#[test]
fn a_file_that_cannot_be_read_is_moved_aside_and_the_ledger_starts_again() {
    let path = scratch("corrupt");
    std::fs::write(&path, "not json").unwrap();
    assert!(FileUsageLedger.transact(&path, &mut |book| book.begin("s", "/p", 1)));
    assert!(path.with_extension("json.bad").exists());
    assert!(FileUsageLedger.read(&path).unwrap().sessions.contains_key("s"));
}

#[test]
fn an_unknown_provider_in_the_file_is_skipped_and_missing_fields_default() {
    let path = scratch("unknown");
    std::fs::write(&path, r#"{"sessions":{"s":{"spent":[{"ts_ms":1,"writer":"w","provider":"nobody","model":"m"},{"ts_ms":2,"writer":"w","provider":"claude","model":"m","input_tokens":3}]}}}"#).unwrap();
    let book = FileUsageLedger.read(&path).unwrap();
    assert_eq!(book.sessions["s"].spent.len(), 1);
    assert_eq!(book.sessions["s"].spent[0].usage.input_tokens, 3);
}

fn account(path: &Path, role_planner: bool, pid: u32) -> UsageAccount {
    let live = Arc::new(KnownProcesses::new(pid));
    if role_planner {
        UsageAccount::planner(Arc::new(FileUsageLedger), live, path.to_path_buf(), Path::new("/repo"), 1_000).with_clock(|| 1_000)
    } else {
        UsageAccount::worker(Arc::new(FileUsageLedger), live, path.to_path_buf(), Path::new("/repo"), 1_000).with_clock(|| 1_000)
    }
}

fn hold_of(account: &UsageAccount, budget: &Budget, tokens: u64) -> Result<jev_orchestration::application::ports::BudgetHold, String> {
    let estimate = Estimate { tokens, usd: None };
    account.hold(budget, &PriceTable::default(), &Fundings::default(), ProviderId::DeepSeek, "deepseek-flash", estimate)
}

#[test]
fn concurrent_reservations_through_separate_handles_cannot_oversubscribe_the_cap() {
    let path = scratch("threads");
    let budget = Budget::capped(Cap::Tokens(10_000));
    let planner = account(&path, true, 100);
    let admitted = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        // Each thread is its own handle, as each process is; one is the planner's own.
        for n in 0..24 {
            let (path, budget, admitted) = (&path, &budget, &admitted);
            scope.spawn(move || {
                let handle = account(path, false, 200 + n);
                if hold_of(&handle, budget, 1_000).is_ok() {
                    admitted.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(admitted.load(Ordering::SeqCst), 10, "10 x 1000 fills the 10000 cap exactly; the rest are refused");
    let reserved = planner.overview(&PriceTable::default(), &Fundings::default()).session_reserved;
    assert_eq!(reserved.paid_tokens, 10_000);
    assert!(hold_of(&planner, &budget, 1).is_err(), "the planner sees the workers' reservations");
}

#[test]
fn two_handles_on_one_file_see_each_others_settled_spend() {
    let path = scratch("two-handles");
    let budget = Budget::capped(Cap::Tokens(1_000));
    let (planner, worker) = (account(&path, true, 1), account(&path, false, 2));
    let hold = hold_of(&worker, &budget, 300).unwrap();
    let used = Usage { input_tokens: 800, ..Usage::default() };
    worker.settle(&hold, ProviderId::DeepSeek, "deepseek-flash", &used);
    assert_eq!(planner.others(&PriceTable::default(), &Fundings::default()).paid().1, 800);
    assert!(hold_of(&planner, &budget, 300).is_err(), "800 spent by the worker + 300 estimated passes 1000");
    assert!(hold_of(&planner, &budget, 100).is_ok());
}

#[test]
fn a_reservation_of_a_dead_process_is_reaped_into_unreported_spend_when_the_next_attempt_asks() {
    let path = scratch("reap");
    let budget = Budget::capped(Cap::Tokens(10_000));
    let (planner, worker) = (account(&path, true, 1), account(&path, false, 2));
    hold_of(&worker, &budget, 9_000).unwrap();
    assert!(hold_of(&planner, &budget, 2_000).is_err(), "the worker's 9000 is still reserved");

    // The same planner session, now told that process 2 is gone.
    let live = Arc::new(KnownProcesses::new(1));
    live.die(2);
    let planner = UsageAccount::planner(Arc::new(FileUsageLedger), live, path.clone(), Path::new("/repo"), 1_000).with_clock(|| 1_000);
    let why = hold_of(&planner, &budget, 2_000).unwrap_err();
    assert!(why.contains("did not report"), "the dead attempt's use is unknown, so the cap fails closed: {why}");
    let book = FileUsageLedger.read(&path).unwrap();
    assert_eq!(book.reserved_all().paid_tokens, 0, "its reservation no longer blocks anyone");
    assert_eq!(planner.others(&PriceTable::default(), &Fundings::default()).unreported_paid_turns(), 1);
}

#[test]
fn a_plan_attempt_is_admitted_while_the_paid_cap_refuses() {
    let path = scratch("plan");
    let budget = Budget::capped(Cap::Tokens(1));
    let planner = account(&path, true, 1);
    assert!(hold_of(&planner, &budget, 10).is_err());
    let hold = planner.hold(&budget, &PriceTable::default(), &Fundings::default(), ProviderId::Claude, "claude-sonnet-5-5", Estimate { tokens: 10, usd: None }).unwrap();
    assert!(hold.id.is_some(), "a plan attempt is reserved too, so it shows as in flight");
    assert_eq!(planner.overview(&PriceTable::default(), &Fundings::default()).session_reserved.attempts, 1);
    planner.release(&hold);
    assert_eq!(planner.overview(&PriceTable::default(), &Fundings::default()).session_reserved.attempts, 0);
}
