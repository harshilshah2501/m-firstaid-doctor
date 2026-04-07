use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    if f.startup_items_count > 20 {
        return Some(Diagnosis {
            issue_class: IssueClass::StartupSlow,
            confidence: Confidence::Confirmed,
            severity: Severity::Medium,
            rule_name: "startup::excessive_items".into(),
            title: format!("Excessive startup items: {} items at login", f.startup_items_count),
            explanation: format!(
                "{} items are configured to launch at login. This is above the recommended \
                 maximum of 10–15. Each item adds to login time and background resource usage. \
                 Review and disable non-essential startup items.",
                f.startup_items_count
            ),
            evidence: vec![
                Evidence::field("startup_items_count", f.startup_items_count).with_threshold(20u32),
            ],
            contributing_ml: None,
        });
    }

    if f.startup_items_count > 12 {
        return Some(Diagnosis {
            issue_class: IssueClass::StartupSlow,
            confidence: Confidence::Probable,
            severity: Severity::Low,
            rule_name: "startup::elevated_items".into(),
            title: format!("Above-average startup items: {} at login", f.startup_items_count),
            explanation: format!(
                "{} items launch at login. While not excessive, reviewing and disabling \
                 unused startup items can improve login speed and reduce idle resource usage.",
                f.startup_items_count
            ),
            evidence: vec![
                Evidence::field("startup_items_count", f.startup_items_count),
            ],
            contributing_ml: None,
        });
    }

    None
}
