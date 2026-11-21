use super::super::interrupt::*;
use std::time::{Duration, Instant};

#[test]
fn a_second_ctrl_c_within_two_seconds_exits() {
    let mut g = InterruptGuard::default();
    let t0 = Instant::now();
    assert_eq!(g.on_interrupt(t0), Interrupt::Warn);
    assert_eq!(g.on_interrupt(t0 + Duration::from_millis(1900)), Interrupt::Exit);
}

#[test]
fn a_pause_longer_than_the_window_starts_over() {
    let mut g = InterruptGuard::default();
    let t0 = Instant::now();
    assert_eq!(g.on_interrupt(t0), Interrupt::Warn);
    assert_eq!(g.on_interrupt(t0 + Duration::from_millis(2500)), Interrupt::Warn);
    assert_eq!(g.on_interrupt(t0 + Duration::from_millis(3000)), Interrupt::Exit, "the window counts from the latest one");
}

#[test]
fn sending_a_message_between_two_ctrl_cs_resets_the_rule() {
    let mut g = InterruptGuard::default();
    let t0 = Instant::now();
    assert_eq!(g.on_interrupt(t0), Interrupt::Warn);
    g.reset();
    assert_eq!(g.on_interrupt(t0 + Duration::from_millis(100)), Interrupt::Warn);
}
