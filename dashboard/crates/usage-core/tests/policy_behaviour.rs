//! Behaviour tests for the pure policy: alerts, freshness and severity, history rules, the
//! headline, the read model, text, settings and the two Claude parsers.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)] // Test fixtures use checked assertions and synthetic values.

use jev_usage_core::alerts::{step, Alert, AlertLevel, AlertMemory};
use jev_usage_core::assess::{
    derive, phase, severity, FreshnessPolicy, Phase, Severity, SourceState,
};
use jev_usage_core::history::{
    downsample, gap_for, range_bounds, reset_marks, should_store, SamplePoint, StoredSample,
    RAW_RETENTION, ROLLUP_RETENTION,
};
use jev_usage_core::ipc::{
    CommandError, HistoryRange, HourCycle, MeterView, ProviderView, ReadingView, RefreshView,
    Snapshot, Theme, UsageView,
};
use jev_usage_core::label::Label;
use jev_usage_core::meter::{
    BalanceReading, LimitStatus, Meter, MeterId, Meters, Reading, WindowKind, WindowReading,
};
use jev_usage_core::parse::{claude_event, claude_log};
use jev_usage_core::port::SourceError;
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::quantity::{Currency, Money, Percent};
use jev_usage_core::settings::{Settings, SettingsPatch, Thresholds};
use jev_usage_core::summary::headline;
use jev_usage_core::text::{self, LocalStamp, LocalTime, TrayRow};
use jev_usage_core::time::{UnixMillis, UnixSeconds};
use jev_usage_core::usage::{Setup, StaleCause, Unavailable, Usage};
use jev_usage_core::view::{self, ProviderState};
use std::time::Duration;

// ---------- builders ----------

fn bp(v: u32) -> Percent {
    Percent::from_basis_points(v)
}

fn window(kind: WindowKind, used_bp: u32, resets_at: Option<u64>) -> Meter {
    Meter::Window(WindowReading {
        kind,
        used: bp(used_bp),
        resets_at: resets_at.map(UnixSeconds),
        status: LimitStatus::Allowed,
    })
}

fn balance(usable: bool) -> Meter {
    Meter::Balance(BalanceReading {
        currency: Currency::Cny,
        total: Money::parse_decimal("12.5").unwrap(),
        granted: Some(Money::parse_decimal("2").unwrap()),
        topped_up: None,
        usable,
    })
}

fn reading(meters: Vec<Meter>, observed_ms: u64) -> Reading {
    Reading {
        meters: Meters::new(meters).unwrap(),
        plan: Some(Label::parse("max_5x").unwrap()),
        observed_at: UnixMillis(observed_ms),
    }
}

fn five_hour(used_bp: u32, resets_at: u64, observed_ms: u64) -> Reading {
    reading(
        vec![window(WindowKind::FiveHour, used_bp, Some(resets_at))],
        observed_ms,
    )
}

// ---------- alerts ----------

const NOW_MS: u64 = 500_000;
const FAR_RESET: u64 = 100_000;

fn alerts_for(
    memory: &AlertMemory,
    used_bp: u32,
    settings: &Settings,
) -> (AlertMemory, Vec<Alert>) {
    step(
        memory,
        ProviderKey::Claude,
        &five_hour(used_bp, FAR_RESET, NOW_MS),
        UnixMillis(NOW_MS),
        settings,
    )
}

#[test]
fn crossing_warn_alerts_once_and_is_remembered() {
    let settings = Settings::defaults();
    let (memory, alerts) = alerts_for(&AlertMemory::default(), 8_500, &settings);
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].level, AlertLevel::Warn);
    assert_eq!(alerts[0].provider, ProviderKey::Claude);
    assert_eq!(alerts[0].window, WindowKind::FiveHour);
    assert_eq!(alerts[0].used, bp(8_500));
    assert_eq!(alerts[0].resets_at, Some(UnixSeconds(FAR_RESET)));
    assert_eq!(memory.entries.len(), 1);
    assert_eq!(memory.entries[0].fired_at, UnixSeconds(500));

    let (again, repeated) = alerts_for(&memory, 8_600, &settings);
    assert!(repeated.is_empty(), "a remembered crossing is not repeated");
    assert_eq!(again, memory);
}

#[test]
fn crossing_both_levels_in_one_step_shows_only_the_higher() {
    let settings = Settings::defaults();
    let (memory, alerts) = alerts_for(&AlertMemory::default(), 9_600, &settings);
    assert_eq!(memory.entries.len(), 2);
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].level, AlertLevel::Critical);
}

#[test]
fn a_level_re_arms_only_after_dropping_below_the_hysteresis_band() {
    let settings = Settings::defaults();
    let (m1, a1) = alerts_for(&AlertMemory::default(), 8_500, &settings);
    assert_eq!(a1.len(), 1);
    // 7 800 is below warn (8 000) but inside the 500 bp band: the crossing is kept.
    let (m2, a2) = alerts_for(&m1, 7_800, &settings);
    assert!(a2.is_empty());
    assert_eq!(m2.entries.len(), 1);
    let (m3, a3) = alerts_for(&m2, 8_500, &settings);
    assert!(a3.is_empty());
    // 7 000 is below the band: the entry is dropped and the next crossing alerts again.
    let (m4, a4) = alerts_for(&m3, 7_000, &settings);
    assert!(a4.is_empty());
    assert!(m4.entries.is_empty());
    let (_, a5) = alerts_for(&m4, 8_500, &settings);
    assert_eq!(a5.len(), 1);
}

#[test]
fn a_new_window_instance_re_arms_the_alert() {
    let settings = Settings::defaults();
    let first = step(
        &AlertMemory::default(),
        ProviderKey::Codex,
        &five_hour(8_500, 1_000, 500_000),
        UnixMillis(500_000),
        &settings,
    );
    assert_eq!(first.1.len(), 1);
    // The old window ended at 1 000 s; 1 100 s later a new one (resetting at 20 000 s) is just as full.
    let second = step(
        &first.0,
        ProviderKey::Codex,
        &five_hour(8_500, 20_000, 1_100_000),
        UnixMillis(1_100_000),
        &settings,
    );
    assert_eq!(second.1.len(), 1, "the new window alerts again");
    assert_eq!(
        second.0.entries[0].window_resets_at,
        Some(UnixSeconds(20_000))
    );
}

#[test]
fn alerts_respect_hidden_providers_age_and_switches() {
    let mut settings = Settings::defaults();
    settings.hidden_providers = vec![ProviderKey::Claude];
    let (memory, alerts) = alerts_for(&AlertMemory::default(), 9_900, &settings);
    assert!(alerts.is_empty() && memory.entries.is_empty());

    // A reading older than ten minutes alerts nothing and records nothing.
    let stale = step(
        &AlertMemory::default(),
        ProviderKey::Claude,
        &five_hour(9_900, 2_000_000, 100),
        UnixMillis(700_000),
        &Settings::defaults(),
    );
    assert!(stale.1.is_empty() && stale.0.entries.is_empty());

    // Notifications off: the crossing is recorded (no later burst) but nothing is shown.
    let mut quiet = Settings::defaults();
    quiet.notifications = false;
    let (memory, alerts) = alerts_for(&AlertMemory::default(), 9_900, &quiet);
    assert!(alerts.is_empty());
    assert_eq!(memory.entries.len(), 2);

    // Critical alerts off: a critical crossing shows nothing even though warn is on.
    let mut no_critical = Settings::defaults();
    no_critical.notify_critical = false;
    let (_, alerts) = alerts_for(&AlertMemory::default(), 9_900, &no_critical);
    assert!(alerts.is_empty());
}

#[test]
fn elapsed_windows_and_balances_never_alert() {
    let settings = Settings::defaults();
    let elapsed = step(
        &AlertMemory::default(),
        ProviderKey::Claude,
        &five_hour(9_900, 400, NOW_MS),
        UnixMillis(NOW_MS),
        &settings,
    );
    assert!(elapsed.1.is_empty() && elapsed.0.entries.is_empty());
    let only_balance = step(
        &AlertMemory::default(),
        ProviderKey::DeepSeek,
        &reading(vec![balance(false)], NOW_MS),
        UnixMillis(NOW_MS),
        &settings,
    );
    assert!(only_balance.1.is_empty() && only_balance.0.entries.is_empty());
}

// ---------- freshness, phase, severity ----------

fn policy() -> FreshnessPolicy {
    FreshnessPolicy {
        fresh_for: Duration::from_mins(1),
    }
}

fn state(
    last_good: Option<Reading>,
    last_failure: Option<(UnixMillis, SourceError)>,
) -> SourceState {
    SourceState {
        last_good,
        last_failure,
        attempted: true,
    }
}

#[test]
fn derive_with_a_good_reading() {
    let good = five_hour(1_000, 9_999_999, 1_000_000);
    // Within the freshness window: known.
    assert_eq!(
        derive(
            &state(Some(good.clone()), None),
            UnixMillis(1_060_000),
            &policy()
        ),
        Usage::Known(good.clone())
    );
    // Past it: stale because quiet, since the moment it stopped being fresh.
    assert_eq!(
        derive(
            &state(Some(good.clone()), None),
            UnixMillis(1_060_001),
            &policy()
        ),
        Usage::Stale {
            last: good.clone(),
            since: UnixMillis(1_060_000),
            cause: StaleCause::Quiet,
        }
    );
    // A newer failure of the unavailable kind: stale because failing.
    let failing = state(
        Some(good.clone()),
        Some((
            UnixMillis(1_030_000),
            SourceError::Unavailable(Unavailable::Offline),
        )),
    );
    assert_eq!(
        derive(&failing, UnixMillis(1_040_000), &policy()),
        Usage::Stale {
            last: good.clone(),
            since: UnixMillis(1_030_000),
            cause: StaleCause::Failing {
                reason: Unavailable::Offline
            },
        }
    );
    // `since` never exceeds the moment the reading stopped being fresh.
    let late = state(
        Some(good.clone()),
        Some((
            UnixMillis(1_900_000),
            SourceError::Unavailable(Unavailable::Io),
        )),
    );
    assert_eq!(
        derive(&late, UnixMillis(2_000_000), &policy()),
        Usage::Stale {
            last: good.clone(),
            since: UnixMillis(1_060_000),
            cause: StaleCause::Failing {
                reason: Unavailable::Io
            },
        }
    );
    // A failure older than the reading, or one that is not "unavailable", is ignored.
    let old = state(
        Some(good.clone()),
        Some((
            UnixMillis(900_000),
            SourceError::Unavailable(Unavailable::Io),
        )),
    );
    assert_eq!(
        derive(&old, UnixMillis(1_010_000), &policy()),
        Usage::Known(good.clone())
    );
    let unsupported = state(
        Some(good.clone()),
        Some((UnixMillis(1_020_000), SourceError::Unsupported)),
    );
    assert_eq!(
        derive(&unsupported, UnixMillis(1_030_000), &policy()),
        Usage::Known(good)
    );
}

#[test]
fn derive_without_a_good_reading() {
    let now = UnixMillis(1);
    assert_eq!(derive(&state(None, None), now, &policy()), Usage::Pending);
    assert_eq!(
        derive(
            &state(
                None,
                Some((now, SourceError::Unavailable(Unavailable::NotLoggedIn)))
            ),
            now,
            &policy()
        ),
        Usage::Unavailable {
            reason: Unavailable::NotLoggedIn
        }
    );
    assert_eq!(
        derive(
            &state(
                None,
                Some((now, SourceError::NotConfigured(Setup::UseGateway)))
            ),
            now,
            &policy()
        ),
        Usage::NotConfigured {
            setup: Setup::UseGateway
        }
    );
    assert_eq!(
        derive(
            &state(None, Some((now, SourceError::Unsupported))),
            now,
            &policy()
        ),
        Usage::Unsupported
    );
}

#[test]
fn phase_and_severity_follow_the_reset_time_and_thresholds() {
    let now = UnixMillis(10_000_000);
    let t = Thresholds::new(bp(8_000), bp(9_500)).unwrap();
    let w = |used, resets| match window(WindowKind::SevenDay, used, resets) {
        Meter::Window(w) => w,
        Meter::Balance(_) => unreachable!(),
    };
    assert_eq!(phase(&w(100, None), now), Phase::Current);
    assert_eq!(phase(&w(100, Some(10_001)), now), Phase::Current);
    assert_eq!(phase(&w(100, Some(10_000)), now), Phase::Elapsed);
    assert_eq!(phase(&w(100, Some(5)), now), Phase::Elapsed);

    let sev = |m: &Meter| severity(m, now, &t);
    assert_eq!(
        sev(&window(WindowKind::FiveHour, 7_999, None)),
        Severity::Ok
    );
    assert_eq!(
        sev(&window(WindowKind::FiveHour, 8_000, None)),
        Severity::Warn
    );
    assert_eq!(
        sev(&window(WindowKind::FiveHour, 9_499, None)),
        Severity::Warn
    );
    assert_eq!(
        sev(&window(WindowKind::FiveHour, 9_500, None)),
        Severity::Critical
    );
    assert_eq!(
        sev(&window(WindowKind::FiveHour, 9_900, Some(10_000))),
        Severity::Unknown
    );
    assert_eq!(sev(&balance(true)), Severity::Ok);
    assert_eq!(sev(&balance(false)), Severity::Critical);
}

// ---------- history rules ----------

fn sample(at: u64, value: i64, resets_at: Option<u64>) -> StoredSample {
    StoredSample {
        at: UnixSeconds(at),
        value,
        resets_at: resets_at.map(UnixSeconds),
    }
}

#[test]
fn should_store_rules() {
    let last = sample(1_000, 5, Some(9_000));
    assert!(should_store(None, &last));
    assert!(!should_store(Some(&last), &sample(1_500, 5, Some(9_000))));
    assert!(should_store(Some(&last), &sample(1_500, 6, Some(9_000))));
    assert!(should_store(Some(&last), &sample(1_500, 5, Some(9_500))));
    assert!(should_store(Some(&last), &sample(1_900, 5, Some(9_000))));
    // Never out of order or repeated in time, whatever changed.
    assert!(!should_store(Some(&last), &sample(1_000, 99, None)));
    assert!(!should_store(Some(&last), &sample(500, 99, None)));
}

fn point(i: i64) -> SamplePoint {
    SamplePoint {
        at: UnixSeconds(u64::try_from(i).unwrap()),
        value: i,
        lo: i - 1,
        hi: i + 1,
    }
}

#[test]
fn downsample_keeps_ends_and_bucket_extremes() {
    let points: Vec<SamplePoint> = (0..10).map(point).collect();
    assert_eq!(downsample(points.clone(), 10), points);
    assert_eq!(downsample(points.clone(), 50), points);
    assert!(downsample(points.clone(), 0).is_empty());
    assert_eq!(downsample(points.clone(), 1), vec![points[0]]);
    // Two points: first and last, no buckets.
    assert_eq!(downsample(points.clone(), 2), vec![points[0], points[9]]);

    let reduced = downsample(points.clone(), 4);
    assert_eq!(reduced.len(), 4);
    assert_eq!(reduced[0], points[0]);
    assert_eq!(reduced[3], points[9]);
    // Bucket 1 covers points 1..5: latest value 4, lowest lo 0, highest hi 5.
    assert_eq!((reduced[1].value, reduced[1].lo, reduced[1].hi), (4, 0, 5));
    // Bucket 2 covers points 5..9: latest value 8, lowest lo 4, highest hi 9.
    assert_eq!((reduced[2].value, reduced[2].lo, reduced[2].hi), (8, 4, 9));
}

#[test]
fn reset_marks_thin_by_window_length_and_give_up_when_noisy() {
    let at = |s: u64| UnixSeconds(s);
    // Five-hour windows: marks closer than 300 * 30 = 9 000 s collapse into the first.
    assert_eq!(
        reset_marks(&[at(0), at(8_999), at(9_000)], &WindowKind::FiveHour),
        vec![at(0), at(9_000)]
    );
    // Every window kind has its own distance.
    let spaced = [at(0), at(2_000_000)];
    for kind in [
        WindowKind::SevenDay,
        WindowKind::Monthly,
        WindowKind::Rolling { minutes: 90 },
        WindowKind::Named {
            label: Label::parse("opus").unwrap(),
        },
    ] {
        assert_eq!(reset_marks(&spaced, &kind).len(), 2, "{kind:?}");
    }
    assert_eq!(
        reset_marks(&[at(0), at(1)], &WindowKind::Rolling { minutes: 0 }).len(),
        2,
        "a zero-length window still has a one-second distance"
    );
    // More than 64 marks means the resets are noise: none are shown.
    let noisy: Vec<UnixSeconds> = (0..70).map(|i| at(i * 1_000_000)).collect();
    assert!(reset_marks(&noisy, &WindowKind::FiveHour).is_empty());
}

#[test]
fn gap_and_range_helpers() {
    assert_eq!(gap_for(Duration::from_mins(1)), Duration::from_mins(75));
    // A long bucket: 2.5 buckets.
    assert_eq!(gap_for(Duration::from_hours(4)), Duration::from_hours(10));
    let now = UnixSeconds(10_000_000);
    for (range, days) in [
        (HistoryRange::Day, 1),
        (HistoryRange::Week, 7),
        (HistoryRange::Month, 30),
        (HistoryRange::Quarter, 90),
    ] {
        assert_eq!(
            range_bounds(range, now),
            (UnixSeconds(10_000_000 - days * 86_400), now)
        );
    }
    assert_eq!(RAW_RETENTION, Duration::from_hours(45 * 24));
    assert_eq!(ROLLUP_RETENTION, Duration::from_hours(400 * 24));
}

// ---------- headline and read model ----------

fn known(used_bp: u32, resets_at: Option<u64>) -> Usage {
    Usage::Known(reading(
        vec![window(WindowKind::FiveHour, used_bp, resets_at)],
        1_000,
    ))
}

#[test]
fn headline_picks_the_fullest_visible_window() {
    let settings = Settings::defaults();
    let now = UnixMillis(2_000);
    let low = known(3_000, Some(9_999));
    let high = known(9_700, Some(9_999));
    let stale = Usage::Stale {
        last: reading(vec![window(WindowKind::SevenDay, 9_000, None)], 1_000),
        since: UnixMillis(1_500),
        cause: StaleCause::Quiet,
    };
    let pending = Usage::Pending;
    let balance_only = Usage::Known(reading(vec![balance(true)], 1_000));

    let none = headline(
        &[
            (ProviderKey::Claude, &pending),
            (ProviderKey::Grok, &balance_only),
        ],
        now,
        &settings,
    );
    assert_eq!(none, None);

    let picked = headline(
        &[
            (ProviderKey::Claude, &low),
            (ProviderKey::Codex, &high),
            (ProviderKey::Gemini, &stale),
        ],
        now,
        &settings,
    )
    .unwrap();
    assert_eq!(picked.provider, ProviderKey::Codex);
    assert_eq!(picked.used, bp(9_700));
    assert_eq!(picked.severity, Severity::Critical);
    assert_eq!(picked.meter, MeterId::Window(WindowKind::FiveHour));
    assert_eq!(picked.resets_at, Some(UnixSeconds(9_999)));

    // A stale reading still counts.
    let from_stale = headline(&[(ProviderKey::Gemini, &stale)], now, &settings).unwrap();
    assert_eq!(from_stale.meter, MeterId::Window(WindowKind::SevenDay));
    assert_eq!(from_stale.severity, Severity::Warn);

    // Ties go to the provider that sorts first, whatever the input order.
    let tie_a = headline(
        &[(ProviderKey::Codex, &high), (ProviderKey::Claude, &high)],
        now,
        &settings,
    )
    .unwrap();
    let tie_b = headline(
        &[(ProviderKey::Claude, &high), (ProviderKey::Codex, &high)],
        now,
        &settings,
    )
    .unwrap();
    assert_eq!(tie_a.provider, ProviderKey::Claude);
    assert_eq!(tie_b.provider, ProviderKey::Claude);

    // Hidden providers and elapsed windows are skipped.
    let mut hidden = Settings::defaults();
    hidden.hidden_providers = vec![ProviderKey::Codex];
    let without_codex = headline(
        &[(ProviderKey::Claude, &low), (ProviderKey::Codex, &high)],
        now,
        &hidden,
    )
    .unwrap();
    assert_eq!(without_codex.provider, ProviderKey::Claude);
    let elapsed = known(9_900, Some(1));
    assert_eq!(
        headline(&[(ProviderKey::Claude, &elapsed)], now, &settings),
        None
    );
}

fn refresh() -> RefreshView {
    RefreshView {
        in_flight: false,
        last_attempt_at: Some(1_700),
        next_at: None,
        free_refresh: true,
        live_check: false,
        live_check_error: None,
    }
}

#[test]
fn snapshot_maps_every_usage_state_sorted_by_provider() {
    let settings = Settings::defaults();
    let now = UnixMillis(2_000_000);
    let both = Usage::Known(reading(
        vec![
            window(WindowKind::FiveHour, 4_000, Some(5_000)),
            balance(false),
        ],
        1_999_000,
    ));
    let stale = Usage::Stale {
        last: reading(
            vec![window(WindowKind::SevenDay, 9_600, Some(1))],
            1_000_000,
        ),
        since: UnixMillis(1_500_000),
        cause: StaleCause::Failing {
            reason: Unavailable::Offline,
        },
    };
    let unavailable = Usage::Unavailable {
        reason: Unavailable::Timeout,
    };
    let not_configured = Usage::NotConfigured {
        setup: Setup::AddDeepSeekKey,
    };
    let states = [
        ProviderState {
            provider: ProviderKey::Grok,
            usage: &Usage::Unsupported,
            refresh: refresh(),
        },
        ProviderState {
            provider: ProviderKey::DeepSeek,
            usage: &not_configured,
            refresh: refresh(),
        },
        ProviderState {
            provider: ProviderKey::Codex,
            usage: &stale,
            refresh: refresh(),
        },
        ProviderState {
            provider: ProviderKey::Gemini,
            usage: &unavailable,
            refresh: refresh(),
        },
        ProviderState {
            provider: ProviderKey::Claude,
            usage: &both,
            refresh: refresh(),
        },
        ProviderState {
            provider: ProviderKey::Antigravity,
            usage: &Usage::Pending,
            refresh: refresh(),
        },
    ];
    let snap = view::snapshot(&states, now, &settings);
    assert_eq!(snap.generated_at, 2_000_000);
    let order: Vec<ProviderKey> = snap.providers.iter().map(|p| p.provider).collect();
    assert_eq!(order, ProviderKey::ALL.to_vec());

    let by = |key: ProviderKey| snap.providers.iter().find(|p| p.provider == key).unwrap();
    match &by(ProviderKey::Claude).usage {
        UsageView::Known { reading } => {
            assert_eq!(reading.observed_at, 1_999_000);
            assert_eq!(reading.plan.as_ref().unwrap().as_str(), "max_5x");
            assert!(matches!(
                reading.meters[0],
                MeterView::Window {
                    used_bp: 4_000,
                    resets_at: Some(5_000),
                    severity: Severity::Ok,
                    phase: jev_usage_core::assess::Phase::Current,
                    ..
                }
            ));
            assert!(matches!(
                reading.meters[1],
                MeterView::Balance {
                    total_micros: 12_500_000,
                    granted_micros: Some(2_000_000),
                    topped_up_micros: None,
                    usable: false,
                    severity: Severity::Critical,
                    ..
                }
            ));
        }
        other => panic!("{other:?}"),
    }
    match &by(ProviderKey::Codex).usage {
        UsageView::Stale {
            reading,
            since,
            cause,
        } => {
            assert_eq!(*since, 1_500_000);
            assert_eq!(
                *cause,
                StaleCause::Failing {
                    reason: Unavailable::Offline
                }
            );
            assert!(matches!(
                reading.meters[0],
                MeterView::Window {
                    severity: Severity::Unknown,
                    phase: jev_usage_core::assess::Phase::Elapsed,
                    ..
                }
            ));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        by(ProviderKey::Gemini).usage,
        UsageView::Unavailable {
            reason: Unavailable::Timeout
        }
    ));
    assert!(matches!(
        by(ProviderKey::DeepSeek).usage,
        UsageView::NotConfigured {
            setup: Setup::AddDeepSeekKey
        }
    ));
    assert!(matches!(
        by(ProviderKey::Grok).usage,
        UsageView::Unsupported
    ));
    assert!(matches!(
        by(ProviderKey::Antigravity).usage,
        UsageView::Pending
    ));
    let head = snap.headline.unwrap();
    assert_eq!(head.provider, ProviderKey::Claude);
    assert_eq!(head.used_bp, 4_000);
    assert_eq!(head.resets_at, Some(5_000));
}

#[test]
fn hidden_providers_are_disabled_in_the_snapshot_and_same_content_ignores_time() {
    let mut settings = Settings::defaults();
    settings.hidden_providers = vec![ProviderKey::Claude];
    let usage = known(5_000, None);
    let states = [ProviderState {
        provider: ProviderKey::Claude,
        usage: &usage,
        refresh: refresh(),
    }];
    let a = view::snapshot(&states, UnixMillis(1_000), &settings);
    let b = view::snapshot(&states, UnixMillis(9_000), &settings);
    assert!(!a.providers[0].enabled);
    assert!(
        a.headline.is_none(),
        "a hidden provider is not the headline"
    );
    assert!(view::same_content(&a, &b));

    let visible = view::snapshot(&states, UnixMillis(1_000), &Settings::defaults());
    assert!(!view::same_content(&a, &visible));
    let mut other_headline = visible.clone();
    other_headline.headline = None;
    assert!(!view::same_content(&visible, &other_headline));
}

// ---------- text ----------

/// UTC as the "local" zone, from the civil-date algorithm.
struct Utc;
impl LocalTime for Utc {
    fn local(&self, at: UnixSeconds) -> LocalStamp {
        let seconds = i64::try_from(at.0).unwrap();
        let days = seconds.div_euclid(86_400);
        let rest = seconds.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        LocalStamp {
            year: i32::try_from(year).unwrap(),
            month: u8::try_from(month).unwrap(),
            day: u8::try_from(day).unwrap(),
            weekday: u8::try_from((days + 3).rem_euclid(7)).unwrap(),
            hour: u8::try_from(rest / 3_600).unwrap(),
            minute: u8::try_from(rest % 3_600 / 60).unwrap(),
        }
    }
}

/// 2026-11-25 (a Wednesday) 12:30 UTC.
const NOW_S: u64 = 1_795_609_800;

fn at(offset_s: i64) -> UnixSeconds {
    UnixSeconds(u64::try_from(i64::try_from(NOW_S).unwrap() + offset_s).unwrap())
}

#[test]
fn percent_and_window_names() {
    assert_eq!(text::percent(bp(0)), "0%");
    assert_eq!(text::percent(bp(1)), "<1%");
    assert_eq!(text::percent(bp(99)), "<1%");
    assert_eq!(text::percent(bp(100)), "1%");
    assert_eq!(text::percent(bp(5_050)), "50%");
    assert_eq!(text::window_name(&WindowKind::FiveHour), "5-hour");
    assert_eq!(text::window_name(&WindowKind::SevenDay), "7-day");
    assert_eq!(text::window_name(&WindowKind::Monthly), "30-day");
    assert_eq!(
        text::window_name(&WindowKind::Rolling { minutes: 2_880 }),
        "2-day"
    );
    assert_eq!(
        text::window_name(&WindowKind::Rolling { minutes: 180 }),
        "3-hour"
    );
    assert_eq!(
        text::window_name(&WindowKind::Named {
            label: Label::parse("opus").unwrap()
        }),
        "opus"
    );
}

#[test]
fn reset_absolute_names_the_day_by_distance() {
    let now = at(0);
    let abs = |offset: i64, hour: HourCycle| text::reset_absolute(at(offset), now, &Utc, hour);
    assert_eq!(abs(3_600, HourCycle::H24), "today 13:30");
    assert_eq!(abs(3_600, HourCycle::H12), "today 1:30 PM");
    assert_eq!(abs(-12 * 3_600, HourCycle::H12), "today 12:30 AM");
    assert_eq!(abs(86_400, HourCycle::H24), "tomorrow 12:30");
    // 2026-11-27 is a Friday: two to six days ahead uses the weekday.
    assert_eq!(abs(2 * 86_400, HourCycle::Auto), "Fri 12:30");
    assert_eq!(abs(6 * 86_400, HourCycle::H24), "Tue 12:30");
    // A week or more: the date, with the year only when it differs.
    assert_eq!(abs(7 * 86_400, HourCycle::H24), "2 Dec 12:30");
    assert_eq!(abs(40 * 86_400, HourCycle::H24), "4 Jan 2027 12:30");
}

#[test]
fn countdown_units() {
    let now = at(0);
    assert_eq!(text::countdown(at(-5), now), None);
    assert_eq!(text::countdown(at(0), now), None);
    assert_eq!(
        text::countdown(at(45), now).as_deref(),
        Some("under a minute")
    );
    assert_eq!(text::countdown(at(60), now).as_deref(), Some("1m"));
    assert_eq!(
        text::countdown(at(59 * 60 + 59), now).as_deref(),
        Some("59m")
    );
    assert_eq!(text::countdown(at(3_600), now).as_deref(), Some("1h 00m"));
    assert_eq!(
        text::countdown(at(2 * 3_600 + 14 * 60), now).as_deref(),
        Some("2h 14m")
    );
    assert_eq!(
        text::countdown(at(172_799), now).as_deref(),
        Some("47h 59m")
    );
    assert_eq!(
        text::countdown(at(172_800 + 13 * 3_600), now).as_deref(),
        Some("2d 13h")
    );
}

#[test]
fn tray_reset_and_stale_stamp() {
    let now = at(0);
    let tray = |offset: i64| text::tray_reset(at(offset), now, &Utc, HourCycle::H24);
    assert_eq!(tray(3_600), "13:30");
    assert_eq!(tray(86_400), "Thu 12:30");
    assert_eq!(tray(5 * 86_400), "Mon 12:30");
    assert_eq!(tray(6 * 86_400), "1 Dec");
    assert_eq!(tray(-3_600), "25 Nov", "a reset in the past shows its date");
    assert_eq!(tray(40 * 86_400), "4 Jan 2027");
    assert_eq!(
        text::stale_stamp(at(-86_400), &Utc, HourCycle::H12),
        "24 Nov 12:30 PM"
    );
}

fn view_window(used_bp: u16, resets_at: Option<u64>, elapsed: bool) -> MeterView {
    MeterView::Window {
        id: MeterId::Window(WindowKind::FiveHour),
        window: WindowKind::FiveHour,
        used_bp,
        resets_at,
        status: LimitStatus::Allowed,
        severity: Severity::Ok,
        phase: if elapsed {
            Phase::Elapsed
        } else {
            Phase::Current
        },
    }
}

fn provider_view(provider: ProviderKey, enabled: bool, usage: UsageView) -> ProviderView {
    ProviderView {
        provider,
        enabled,
        usage,
        refresh: refresh(),
    }
}

fn reading_view(meters: Vec<MeterView>) -> ReadingView {
    ReadingView {
        observed_at: (NOW_S - 86_400) * 1_000,
        plan: None,
        meters,
    }
}

#[test]
fn tray_rows_for_every_state() {
    let now = at(0);
    let rows_of = |providers: Vec<ProviderView>| -> Vec<String> {
        let snap = Snapshot {
            generated_at: 0,
            providers,
            headline: None,
        };
        text::tray_rows(&snap, now, &Utc, HourCycle::H24)
            .into_iter()
            .map(|TrayRow { text, .. }| text)
            .collect()
    };
    let reasons = [
        (Unavailable::NotInstalled, "not installed"),
        (Unavailable::NotLoggedIn, "not signed in"),
        (Unavailable::Offline, "offline"),
        (Unavailable::Timeout, "timed out"),
        (Unavailable::RateLimited, "rate limited"),
        (Unavailable::Unauthorized, "sign-in required"),
        (Unavailable::ServerError, "server error"),
        (Unavailable::Protocol, "unexpected response"),
        (Unavailable::Io, "could not read data"),
        (Unavailable::Internal, "something went wrong"),
    ];
    for (reason, phrase) in reasons {
        assert_eq!(
            rows_of(vec![provider_view(
                ProviderKey::Codex,
                true,
                UsageView::Unavailable { reason }
            )]),
            vec![format!("Codex · {phrase}")]
        );
    }
    assert_eq!(
        rows_of(vec![
            provider_view(ProviderKey::Claude, true, UsageView::Pending),
            provider_view(
                ProviderKey::Gemini,
                true,
                UsageView::NotConfigured {
                    setup: Setup::UseGateway
                }
            ),
            provider_view(
                ProviderKey::DeepSeek,
                true,
                UsageView::NotConfigured {
                    setup: Setup::AddDeepSeekKey
                }
            ),
            provider_view(ProviderKey::Grok, true, UsageView::Unsupported),
            provider_view(ProviderKey::Antigravity, false, UsageView::Pending),
        ]),
        vec![
            "Claude · checking".to_owned(),
            "Gemini · use gateway to start".to_owned(),
            "DeepSeek · add DeepSeek key".to_owned(),
        ],
        "unsupported providers have no row; disabled ones are skipped"
    );

    let known = |meters| UsageView::Known {
        reading: reading_view(meters),
    };
    assert_eq!(
        rows_of(vec![provider_view(
            ProviderKey::Claude,
            true,
            known(vec![
                view_window(5_000, Some(NOW_S + 3_600), false),
                view_window(20, None, false),
                view_window(9_000, Some(NOW_S - 10), true),
                MeterView::Balance {
                    id: MeterId::Balance(Currency::Cny),
                    currency: Currency::Cny,
                    total_micros: 1,
                    granted_micros: None,
                    topped_up_micros: None,
                    usable: true,
                    severity: Severity::Ok,
                },
            ])
        )]),
        vec![
            "Claude · 5-hour 50% · resets 13:30".to_owned(),
            "Claude · 5-hour <1%".to_owned(),
            "Claude · 5-hour · no data since reset".to_owned(),
        ]
    );
    let stale = UsageView::Stale {
        reading: reading_view(vec![view_window(7_000, Some(NOW_S + 60), false)]),
        since: 0,
        cause: StaleCause::Quiet,
    };
    assert_eq!(
        rows_of(vec![provider_view(ProviderKey::Codex, true, stale)]),
        vec!["Codex · 5-hour 70% · as of 24 Nov 12:30".to_owned()]
    );
}

#[test]
fn notification_text() {
    let now = at(0);
    let alert = |resets_at| Alert {
        provider: ProviderKey::Claude,
        window: WindowKind::SevenDay,
        level: AlertLevel::Warn,
        used: bp(8_250),
        resets_at,
    };
    assert_eq!(
        text::notification(&alert(Some(at(86_400))), now, &Utc, HourCycle::H24),
        (
            "Claude 7-day at 82%".to_owned(),
            "Resets tomorrow 12:30.".to_owned()
        )
    );
    assert_eq!(
        text::notification(&alert(None), now, &Utc, HourCycle::H24),
        ("Claude 7-day at 82%".to_owned(), String::new())
    );
}

// ---------- settings ----------

#[test]
fn validated_clamps_snaps_and_resets_unknown_versions() {
    let mut old = Settings::defaults();
    old.version = 0;
    old.warn_bp = 5_100;
    assert_eq!(old.validated(), Settings::defaults());

    let mut s = Settings::defaults();
    s.warn_bp = 100;
    s.critical_bp = 100;
    s.zoom_pct = 160;
    let v = s.validated();
    assert_eq!((v.warn_bp, v.critical_bp, v.zoom_pct), (5_000, 5_001, 150));

    let mut high = Settings::defaults();
    high.warn_bp = 60_000;
    high.critical_bp = 60_000;
    high.zoom_pct = 900;
    let v = high.validated();
    assert_eq!((v.warn_bp, v.critical_bp, v.zoom_pct), (9_999, 10_000, 200));

    let mut inverted = Settings::defaults();
    inverted.warn_bp = 9_000;
    inverted.critical_bp = 6_000;
    let v = inverted.validated();
    assert_eq!((v.warn_bp, v.critical_bp), (9_000, 9_001));
}

#[test]
fn apply_sets_every_field_and_rejects_invalid_combinations() {
    let base = Settings::defaults();
    let patch = SettingsPatch {
        warn_bp: Some(7_000),
        critical_bp: Some(9_000),
        notifications: Some(false),
        notify_warn: Some(false),
        notify_critical: Some(false),
        theme: Some(Theme::Dark),
        hour_cycle: Some(HourCycle::H24),
        zoom_pct: Some(175),
        close_to_tray: Some(false),
        autostart: Some(false),
        shortcuts: Some(false),
        claude_live_check: Some(true),
        first_run_seen: Some(true),
        hidden_providers: Some(vec![ProviderKey::Grok]),
    };
    let next = base.apply(patch).unwrap();
    assert_eq!(
        next,
        Settings {
            version: base.version,
            warn_bp: 7_000,
            critical_bp: 9_000,
            notifications: false,
            notify_warn: false,
            notify_critical: false,
            theme: Theme::Dark,
            hour_cycle: HourCycle::H24,
            zoom_pct: 175,
            close_to_tray: false,
            autostart: false,
            shortcuts: false,
            claude_live_check: true,
            first_run_seen: true,
            hidden_providers: vec![ProviderKey::Grok],
        }
    );
    assert_eq!(base.apply(SettingsPatch::default()).unwrap(), base);

    let bad = |patch: SettingsPatch| base.apply(patch).unwrap_err();
    assert_eq!(
        bad(SettingsPatch {
            warn_bp: Some(4_999),
            ..SettingsPatch::default()
        }),
        CommandError::InvalidArgument
    );
    assert_eq!(
        bad(SettingsPatch {
            critical_bp: Some(10_001),
            ..SettingsPatch::default()
        }),
        CommandError::InvalidArgument
    );
    assert_eq!(
        bad(SettingsPatch {
            warn_bp: Some(9_600),
            ..SettingsPatch::default()
        }),
        CommandError::InvalidArgument
    );
    assert_eq!(
        bad(SettingsPatch {
            warn_bp: Some(9_000),
            critical_bp: Some(9_000),
            ..SettingsPatch::default()
        }),
        CommandError::InvalidArgument
    );
    assert_eq!(
        bad(SettingsPatch {
            zoom_pct: Some(110),
            ..SettingsPatch::default()
        }),
        CommandError::InvalidArgument
    );
}

#[test]
fn thresholds_come_from_settings_with_a_safe_fallback() {
    let t = Settings::defaults().thresholds();
    assert_eq!((t.warn(), t.critical()), (bp(8_000), bp(9_500)));
    let mut custom = Settings::defaults();
    custom.warn_bp = 6_000;
    custom.critical_bp = 7_000;
    let t = custom.thresholds();
    assert_eq!((t.warn(), t.critical()), (bp(6_000), bp(7_000)));
    // Settings that bypassed validation fall back to the defaults.
    let mut broken = Settings::defaults();
    broken.warn_bp = 9_000;
    broken.critical_bp = 9_000;
    let t = broken.thresholds();
    assert_eq!((t.warn(), t.critical()), (bp(8_000), bp(9_500)));
    assert!(Thresholds::new(bp(5_000), bp(5_000)).is_none());
}

// ---------- Claude parsers ----------

fn event(five: &str, seven: &str) -> Vec<u8> {
    format!(
        r#"{{"type":"rate_limit_event","rate_limit_info":{{"unifiedWindows":{{{five}{seven}}}}}}}"#
    )
    .into_bytes()
}

#[test]
fn claude_event_maps_windows_and_statuses() {
    let line = event(
        r#""five_hour":{"utilization":0.5,"resetsAt":1795000000,"status":"allowed_warning"},"#,
        r#""seven_day":{"utilization":0.25,"status":"rejected"}"#,
    );
    let meters = claude_event::parse_line(&line).unwrap();
    let all = meters.as_slice();
    assert_eq!(all.len(), 2);
    match &all[0] {
        Meter::Window(w) => {
            assert_eq!(w.kind, WindowKind::FiveHour);
            assert_eq!(w.used, bp(5_000));
            assert_eq!(w.resets_at, Some(UnixSeconds(1_795_000_000)));
            assert_eq!(w.status, LimitStatus::Warning);
        }
        Meter::Balance(_) => panic!("window expected"),
    }
    match &all[1] {
        Meter::Window(w) => {
            assert_eq!(w.kind, WindowKind::SevenDay);
            assert_eq!(w.resets_at, None);
            assert_eq!(w.status, LimitStatus::Rejected);
        }
        Meter::Balance(_) => panic!("window expected"),
    }
    let allowed = event(r#""five_hour":{"utilization":0.1,"status":"allowed"}"#, "");
    assert!(matches!(
        &claude_event::parse_line(&allowed).unwrap().as_slice()[0],
        Meter::Window(w) if w.status == LimitStatus::Allowed
    ));
    let odd = event(
        r#""five_hour":{"utilization":0.1,"status":"something_else"}"#,
        "",
    );
    assert!(matches!(
        &claude_event::parse_line(&odd).unwrap().as_slice()[0],
        Meter::Window(w) if w.status == LimitStatus::Unknown
    ));
    let none = event(r#""five_hour":{"utilization":0.1}"#, "");
    assert!(matches!(
        &claude_event::parse_line(&none).unwrap().as_slice()[0],
        Meter::Window(w) if w.status == LimitStatus::Unknown
    ));
}

#[test]
fn claude_event_rejects_everything_that_is_not_a_usable_event() {
    assert!(claude_event::parse_line(b"not json").is_none());
    assert!(claude_event::parse_line(b"{}").is_none());
    let other = br#"{"type":"assistant","rate_limit_info":{"unifiedWindows":{}}}"#;
    assert!(claude_event::parse_line(other).is_none());
    // No windows at all.
    assert!(claude_event::parse_line(&event("", "")).is_none());
    // Unusable utilisation (negative) drops that window; the other survives.
    let line = event(
        r#""five_hour":{"utilization":-1.0,"status":"allowed"},"#,
        r#""seven_day":{"utilization":0.4}"#,
    );
    let meters = claude_event::parse_line(&line).unwrap();
    assert_eq!(meters.as_slice().len(), 1);
    assert_eq!(
        meters.as_slice()[0].id(),
        MeterId::Window(WindowKind::SevenDay)
    );
    let only_bad = event(r#""five_hour":{"utilization":-1.0}"#, "");
    assert!(claude_event::parse_line(&only_bad).is_none());
}

fn log_row(ratelimit: &str) -> Vec<u8> {
    format!(r#"{{"ts_ms":1794890000000,"cwd":"/x","ratelimit":{{{ratelimit}}}}}"#).into_bytes()
}

const P: &str = "anthropic-ratelimit-unified-";

#[test]
fn claude_log_reads_only_the_allow_listed_headers() {
    let line = log_row(&format!(
        r#""{P}status":"allowed","{P}5h-utilization":"0.41","{P}5h-reset":"1794898800","{P}5h-status":"allowed","{P}7d-utilization":"0.3","{P}7d-status":"allowed_warning","{P}opus-utilization":"0.9","{P}opus-reset":"1795000000","{P}opus-status":"rejected","{P}overage-disabled-reason":"org_level_disabled","{P}fallback-percentage":"0.5","unrelated":"x""#
    ));
    assert!(claude_log::might_have_sample(&line));
    let sample = claude_log::parse_line(&line).unwrap();
    assert_eq!(sample.at, UnixMillis(1_794_890_000_000));
    let kinds: Vec<WindowKind> = sample
        .windows
        .as_slice()
        .iter()
        .map(|m| match m {
            Meter::Window(w) => w.kind.clone(),
            Meter::Balance(_) => panic!("window expected"),
        })
        .collect();
    // Sorted by header name: 5h, 7d, opus.
    assert_eq!(
        kinds,
        vec![
            WindowKind::FiveHour,
            WindowKind::SevenDay,
            WindowKind::Named {
                label: Label::parse("opus").unwrap()
            }
        ]
    );
    let five = match &sample.windows.as_slice()[0] {
        Meter::Window(w) => w.clone(),
        Meter::Balance(_) => panic!("window expected"),
    };
    assert_eq!(five.used, bp(4_100));
    assert_eq!(five.resets_at, Some(UnixSeconds(1_794_898_800)));
    assert_eq!(five.status, LimitStatus::Allowed);
    let seven = match &sample.windows.as_slice()[1] {
        Meter::Window(w) => w.clone(),
        Meter::Balance(_) => panic!("window expected"),
    };
    assert_eq!(
        (seven.resets_at, seven.status),
        (None, LimitStatus::Warning)
    );
}

#[test]
fn claude_log_tolerates_hostile_values_and_shapes() {
    // Non-string header values never become numbers or statuses.
    let line = log_row(&format!(
        r#""{P}5h-utilization":0.5,"{P}5h-reset":true,"{P}5h-status":null,"{P}7d-utilization":"0.2","{P}7d-reset":[1,{{"a":2}}],"{P}7d-status":{{"k":"v"}},"{P}30d-utilization":"0.1","{P}30d-reset":-1,"{P}30d-status":1.5"#
    ));
    let sample = claude_log::parse_line(&line).unwrap();
    // 5h has no usable utilisation and is dropped; 7d and 30d keep what is valid.
    let ids: Vec<MeterId> = sample.windows.as_slice().iter().map(Meter::id).collect();
    assert_eq!(
        ids,
        vec![
            MeterId::Window(WindowKind::Named {
                label: Label::parse("30d").unwrap()
            }),
            MeterId::Window(WindowKind::SevenDay),
        ]
    );
    // Duplicate headers: the first value wins.
    let dup = log_row(&format!(
        r#""{P}5h-utilization":"0.1","{P}5h-utilization":"0.9","{P}5h-reset":"10","{P}5h-reset":"20","{P}5h-status":"allowed","{P}5h-status":"rejected""#
    ));
    let w = match &claude_log::parse_line(&dup).unwrap().windows.as_slice()[0] {
        Meter::Window(w) => w.clone(),
        Meter::Balance(_) => panic!("window expected"),
    };
    assert_eq!(
        (w.used, w.resets_at, w.status),
        (bp(1_000), Some(UnixSeconds(10)), LimitStatus::Allowed)
    );
}

#[test]
fn claude_log_rejects_what_is_not_a_sample() {
    assert!(!claude_log::might_have_sample(b"{\"ts_ms\":1}"));
    assert!(claude_log::parse_line(b"{\"ts_ms\":1}").is_none());
    assert!(
        claude_log::parse_line(b"{\"ratelimit\":{\"x\":1}").is_none(),
        "truncated JSON"
    );
    assert!(claude_log::parse_line(&log_row("")).is_none(), "no windows");
    // A row without ts_ms is not a sample.
    let no_ts = format!(r#"{{"ratelimit":{{"{P}5h-utilization":"0.5"}}}}"#);
    assert!(claude_log::parse_line(no_ts.as_bytes()).is_none());
    // Only the first eight windows are kept.
    let many: String = (0..12)
        .map(|i| format!(r#""{P}w{i:02}-utilization":"0.{i}""#))
        .collect::<Vec<_>>()
        .join(",");
    let capped = claude_log::parse_line(&log_row(&many)).unwrap();
    assert_eq!(capped.windows.as_slice().len(), Meters::MAX);
}
