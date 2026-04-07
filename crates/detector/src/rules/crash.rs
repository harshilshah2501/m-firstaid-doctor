use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    let process_name = f.crash_process_name.as_deref().unwrap_or("unknown");
    let exception_type = f.crash_exception_type.as_deref().unwrap_or("unknown");

    if f.app_crash_count_24h == 0 { return None; }

    let (confidence, severity, title, explanation, rule) = if f.app_crash_count_24h >= 5 {
        (
            Confidence::Confirmed, Severity::High,
            format!("Repeated crash: '{}' crashed {} times in 24h", process_name, f.app_crash_count_24h),
            format!("'{}' has crashed {} times in the past 24 hours. This indicates a \
                     persistent defect, not a transient issue. Exception: {}. \
                     Clearing the app cache or reinstalling may resolve it.",
                     process_name, f.app_crash_count_24h, exception_type),
            "crash::repeated_systematic",
        )
    } else {
        (
            Confidence::Probable, Severity::Medium,
            format!("'{}' has crashed {} times today", process_name, f.app_crash_count_24h),
            format!("'{}' has crashed {} times today with exception type '{}'. \
                     This may be a transient issue or an early sign of a recurring problem.",
                     process_name, f.app_crash_count_24h, exception_type),
            "crash::moderate_frequency",
        )
    };

    Some(Diagnosis {
        issue_class: IssueClass::AppCrash,
        confidence,
        severity,
        rule_name: rule.into(),
        title,
        explanation,
        evidence: vec![
            Evidence::field("app_crash_count_24h", f.app_crash_count_24h).with_threshold(2u32),
            Evidence::field("crash_process_name", process_name),
            Evidence::field("crash_exception_type", exception_type),
            Evidence::field("recent_update_detected", f.recent_update_detected),
        ],
        contributing_ml: None,
    })
}
