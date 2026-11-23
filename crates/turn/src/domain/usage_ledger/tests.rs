use super::*;

fn held(id: &str, pid: u32, ts_ms: u64, paid: bool, tokens: u64) -> Held {
    Held { id: id.into(), owner_pid: pid, owner_start: None, ts_ms, provider: ProviderId::DeepSeek, model: "deepseek-flash".into(), paid, tokens, usd: None }
}

fn spent(ts_ms: u64, writer: &str, tokens: u64) -> Spent {
    Spent { ts_ms, writer: writer.into(), provider: ProviderId::DeepSeek, model: "deepseek-flash".into(), usage: Usage { input_tokens: tokens, ..Usage::default() } }
}

fn book() -> Book {
    let mut book = Book::default();
    book.begin("s1", "/repo", 1_000);
    book
}

#[test]
fn a_reservation_counts_until_it_settles_and_then_the_spend_replaces_it() {
    let mut book = book();
    book.hold("s1", held("h1", 1, 1_100, true, 500));
    assert_eq!(book.sessions["s1"].reserved().paid_tokens, 500);
    book.settle("s1", Some("h1"), spent(1_200, "w", 120));
    let session = &book.sessions["s1"];
    assert!(session.held.is_empty());
    assert_eq!(session.meter(&PriceTable::default(), &Fundings::default(), None).paid().1, 120, "what it really used, not what was reserved");
}

#[test]
fn settling_with_unknown_usage_is_unreported_not_zero() {
    let mut book = book();
    book.hold("s1", held("h1", 1, 1_100, true, 500));
    book.settle("s1", Some("h1"), Spent::unknown(1_200, "w", ProviderId::DeepSeek, "deepseek-flash"));
    let meter = book.sessions["s1"].meter(&PriceTable::default(), &Fundings::default(), None);
    assert_eq!(meter.unreported_paid_turns(), 1);
}

#[test]
fn a_reservation_whose_owner_died_is_reaped_into_unreported_spend() {
    let mut book = book();
    book.hold("s1", held("dead", 7, 1_100, true, 500));
    book.hold("s1", held("alive", 8, 1_100, true, 300));
    assert_eq!(book.reap(2_000, &|h| h.owner_pid != 7), 1);
    let session = &book.sessions["s1"];
    assert_eq!(session.held.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), ["alive"]);
    let meter = session.meter(&PriceTable::default(), &Fundings::default(), None);
    assert_eq!(meter.unreported_paid_turns(), 1, "what the dead attempt spent is unknown, which is not free");
    assert_eq!(session.spent[0].writer, REAPED_WRITER);
}

#[test]
fn a_reservation_older_than_the_limit_is_reaped_even_if_its_pid_looks_alive() {
    let mut book = book();
    book.hold("s1", held("old", 1, 1_100, true, 5));
    assert_eq!(book.reap(1_100 + MAX_HOLD_AGE_MS, &|_| true), 0);
    assert_eq!(book.reap(1_101 + MAX_HOLD_AGE_MS, &|_| true), 1);
}

#[test]
fn releasing_removes_the_reservation_and_records_nothing() {
    let mut book = book();
    book.hold("s1", held("h1", 1, 1_100, true, 500));
    assert_eq!(book.release("h1").as_deref(), Some("s1"));
    assert!(book.sessions["s1"].held.is_empty() && book.sessions["s1"].spent.is_empty());
    assert_eq!(book.release("h1"), None);
}

#[test]
fn a_settle_after_the_reaper_still_records_the_spend() {
    let mut book = book();
    book.settle("s1", Some("gone"), spent(1_200, "w", 40));
    assert_eq!(book.sessions["s1"].spent.len(), 1);
}

#[test]
fn a_worker_joins_the_projects_session_and_starts_one_when_that_went_idle() {
    let mut book = book();
    assert_eq!(book.join("/repo", 2_000, || "w-1".into()), "s1");
    assert_eq!(book.join("/other", 2_000, || "w-2".into()), "w-2", "no planner there: a session of its own");
    let later = 1_000 + SESSION_IDLE_MS;
    assert_eq!(book.join("/repo", later, || "w-3".into()), "w-3", "an idle session is not joined");
    assert_eq!(book.current["/repo"], "w-3");
}

#[test]
fn a_new_planner_session_starts_fresh_and_the_day_still_counts_both() {
    let mut book = book();
    book.settle("s1", None, spent(2_000, "p", 70));
    book.begin("s2", "/repo", 3_000);
    let (prices, fundings) = (PriceTable::default(), Fundings::default());
    assert_eq!(book.sessions["s2"].meter(&prices, &fundings, None).paid().1, 0);
    assert_eq!(book.day_meter(4_000, &prices, &fundings).paid().1, 70, "the daily total spans sessions");
    assert_eq!(book.day_meter(4_000 + DAY_MS, &prices, &fundings).paid().1, 0, "and starts over the next day");
}

#[test]
fn the_meter_can_leave_out_one_writer_and_counts_plan_use_apart_from_paid() {
    let mut book = book();
    book.settle("s1", None, spent(2_000, "me", 70));
    book.settle("s1", None, spent(2_100, "other", 30));
    let (prices, fundings) = (PriceTable::default(), Fundings::default());
    assert_eq!(book.sessions["s1"].meter(&prices, &fundings, Some("me")).paid().1, 30);
    let mut plan = spent(2_200, "other", 9);
    plan.provider = ProviderId::Claude;
    plan.model = "claude-sonnet-5-5".into();
    book.settle("s1", None, plan);
    let meter = book.sessions["s1"].meter(&prices, &fundings, None);
    assert_eq!(meter.paid().1, 100, "plan tokens are not paid money");
    assert_eq!(meter.plan_use(ProviderId::Claude), (9, 0));
}

#[test]
fn old_sessions_fall_away_but_the_current_and_those_with_reservations_stay() {
    let mut book = book();
    book.begin("busy", "/busy", 1_000);
    book.hold("busy", held("h", 1, 1_000, true, 1));
    book.begin("old", "/old", 1_000);
    book.current.remove("/old");
    book.begin("new", "/new", 1_000 + RETAIN_MS + 5);
    assert!(book.sessions.contains_key("new") && book.sessions.contains_key("busy"));
    assert!(!book.sessions.contains_key("old"));
}
