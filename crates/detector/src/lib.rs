pub mod types;
pub mod rules;

pub use types::*;

use feature_extractor::FeatureSet;
use trigger_engine::IssueClass;
use chrono::Utc;
use uuid::Uuid;

/// Run the full deterministic rules engine for all triggered issue classes.
/// ML augmentation (Phase 3) plugs in after this via `merge_ml_confidence`.
pub fn diagnose(features: &FeatureSet, triggered_classes: &[IssueClass]) -> DiagnosisOutput {
    let started = Utc::now();
    let mut diagnoses = Vec::new();

    for class in triggered_classes {
        let result = match class {
            IssueClass::HighCpu      => rules::cpu::diagnose(features),
            IssueClass::HighMemory   => rules::memory::diagnose(features),
            IssueClass::LowDisk      => rules::disk::diagnose(features),
            IssueClass::DiskIo       => rules::disk_io::diagnose(features),
            IssueClass::StartupSlow  => rules::startup::diagnose(features),
            IssueClass::AppCrash     => rules::crash::diagnose(features),
            IssueClass::NetworkDns   => rules::network::diagnose(features),
            IssueClass::ServiceDown  => rules::service::diagnose(features),
            IssueClass::UpdateIssue  => rules::update::diagnose(features),
            IssueClass::BatteryThermal => rules::battery::diagnose(features),
        };

        if let Some(d) = result {
            diagnoses.push(d);
        }
    }

    DiagnosisOutput {
        schema_version: "1.0".into(),
        diagnosis_id: Uuid::new_v4(),
        snapshot_id: features.snapshot_id,
        diagnosed_at: Utc::now(),
        diagnoses,
        detection_method: DetectionMethod::RulesOnly,
        ml_models_invoked: Vec::new(),
        elapsed_ms: (Utc::now() - started).num_milliseconds() as u64,
    }
}
