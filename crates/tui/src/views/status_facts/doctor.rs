//! Owns the mapping from the doctor's report to its screen. Must not run any check.

use jev_doctor::domain::diagnose as doctor;
use crate::render as uifmt;
use crate::render::{DoctorItem, Style};

fn doctor_item(c: &doctor::Check) -> DoctorItem {
    DoctorItem {
        severity: match c.severity {
            doctor::Severity::Ok => uifmt::Severity::Ok,
            doctor::Severity::Info => uifmt::Severity::Info,
            doctor::Severity::Warn => uifmt::Severity::Warn,
            doctor::Severity::Problem => uifmt::Severity::Fail,
        },
        title: c.title.clone(),
        detail: c.detail.clone(),
        fix: c.fix.clone(),
    }
}

/// Whether the report has a Problem: the `jev-code doctor` exit code is 1 then.
pub fn has_problem(report: &doctor::Report) -> bool {
    report.checks.iter().any(|c| c.severity == doctor::Severity::Problem)
}

/// The doctor's screen: findings with the worst first, what routing can do now, and a
/// closing count.
pub fn doctor_text(report: &doctor::Report, style: Style, width: usize) -> String {
    let items: Vec<DoctorItem> = report.checks.iter().map(doctor_item).collect();
    let mut out = uifmt::doctor_report(&items, style, width);
    out.push_str(&format!("\n\n{}\n{}", report.routing_note, uifmt::doctor_summary(&items, style)));
    out
}
