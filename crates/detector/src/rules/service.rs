use crate::types::*;
use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;

pub fn diagnose(f: &FeatureSet) -> Option<Diagnosis> {
    if f.service_restart_storm {
        return Some(Diagnosis {
            issue_class: IssueClass::ServiceDown,
            confidence: Confidence::Confirmed,
            severity: Severity::High,
            rule_name: "service::restart_storm".into(),
            title: "Service restart storm detected — service is crash-looping".into(),
            explanation: "A monitored service has restarted more than 10 times in the past hour. \
                          This indicates a crash-loop: the service starts, fails, and restarts \
                          repeatedly. The repeated restarts consume CPU and may mask the \
                          underlying error. The service should be stopped and logs examined.".into(),
            evidence: vec![
                Evidence::field("service_restart_storm", f.service_restart_storm),
            ],
            contributing_ml: None,
        });
    }

    if f.service_down_expected {
        return Some(Diagnosis {
            issue_class: IssueClass::ServiceDown,
            confidence: Confidence::Confirmed,
            severity: Severity::High,
            rule_name: "service::unexpected_stop".into(),
            title: "Expected service is not running".into(),
            explanation: "One or more services that are configured as expected-running \
                          are currently stopped. This may cause dependent functionality to fail.".into(),
            evidence: vec![
                Evidence::field("service_down_expected", f.service_down_expected),
            ],
            contributing_ml: None,
        });
    }

    None
}
