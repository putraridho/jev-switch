//! Plan utilization readings and their persistence.

use super::*;
use jev_backend::PlanUsage;

    #[test]
    fn plan_utilization_reports_the_latest_reading() {
        let mut rt = runtime();
        assert_eq!(rt.plan_utilization(Claude), None);
        rt.note_plan(Claude, PlanUsage { window: "5h".into(), utilization: 0.4, resets_at: None });
        rt.note_plan(Claude, PlanUsage { window: "5h".into(), utilization: 0.7, resets_at: None });
        assert_eq!(rt.plan_utilization(Claude), Some(0.7));
        assert_eq!(rt.plan_utilization(Codex), None);
        // A window that has already reset says nothing about now.
        rt.note_plan(Codex, PlanUsage { window: "5h".into(), utilization: 0.9, resets_at: Some(1) });
        assert_eq!(rt.plan_utilization(Codex), None);
    }

    #[test]
    fn a_background_gemini_reading_counts_at_once_and_is_kept_after_a_gemini_turn() {
        let mut rt = runtime();
        rt.note_plan(Gemini, PlanUsage { window: "five_hour".into(), utilization: 0.1, resets_at: None });
        *rt.gemini_plan.reading.lock().unwrap() = Some(PlanUsage { window: "seven_day".into(), utilization: 0.6, resets_at: None });
        assert_eq!(rt.plan_utilization(Gemini), Some(0.6));
        assert_eq!(rt.plan.get(&Gemini).map(|p| p.utilization), Some(0.1));
        rt.after_turn(Gemini);
        assert_eq!(rt.plan.get(&Gemini).map(|p| p.utilization), Some(0.6));
    }

    #[test]
    fn new_readings_are_saved_and_a_restart_loads_them() {
        let dir = std::env::temp_dir().join(format!("jev-plan-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plan-usage.json");
        let mut rt = runtime();
        rt.plan_path = Some(path.clone());
        rt.note_plan(Claude, PlanUsage { window: "five_hour".into(), utilization: 0.4, resets_at: None });
        assert_eq!(crate::infrastructure::plan_store::load(&path).get(&Claude).map(|p| p.utilization), Some(0.4));
        // The same reading again does not rewrite the file.
        std::fs::remove_file(&path).unwrap();
        rt.note_plan(Claude, PlanUsage { window: "five_hour".into(), utilization: 0.4, resets_at: None });
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
