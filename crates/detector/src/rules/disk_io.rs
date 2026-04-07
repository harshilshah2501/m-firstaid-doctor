use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    // Rule 1: SMART failure — always highest priority
    if f.smart_failing {
        return Some(Diagnosis {
            issue_class: IssueClass::DiskIo,
            confidence: Confidence::Confirmed,
            severity: Severity::Critical,
            rule_name: "disk_io::smart_failure".into(),
            title: "CRITICAL: SMART reports disk failure — back up immediately".into(),
            explanation: "The disk's S.M.A.R.T. (Self-Monitoring, Analysis and Reporting \
                          Technology) system is reporting a failure condition. This indicates \
                          imminent physical disk failure. Back up all data immediately. \
                          No software fix can address hardware failure.".into(),
            evidence: vec![
                Evidence::field("smart_failing", f.smart_failing),
            ],
            contributing_ml: None,
        });
    }

    None // Disk I/O rules rely heavily on queue depth and latency — fields from platform adapters
}
