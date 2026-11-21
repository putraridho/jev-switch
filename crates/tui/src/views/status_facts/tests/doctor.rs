use super::*;
use jev_doctor::domain::diagnose as doctor;
use crate::render::Style;
use super::super::doctor::{doctor_text, has_problem};

#[test]
fn the_doctor_screen_puts_problems_first_and_ends_with_a_count() {
    let inputs = doctor::Inputs { avail: vec![av(Claude, State::NotLoggedIn)], jev_key_present: false, ..doctor::Inputs::default() };
    let report = doctor::diagnose(&inputs);
    assert!(has_problem(&report));
    let text = doctor_text(&report, Style::off(), 80);
    assert!(text.starts_with("✗ "), "{text}");
    assert!(text.contains("$ claude auth login"), "{text}");
    assert!(text.contains("No provider is ready"), "{text}");
    assert!(text.lines().last().unwrap().contains("problem"), "{text}");

    let fine = doctor::diagnose(&doctor::Inputs { avail: vec![av(Claude, State::Ready), av(Codex, State::Ready)], jev_key_present: true, ..doctor::Inputs::default() });
    assert!(!has_problem(&fine));
}
