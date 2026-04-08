//! ML confidence merger — Phase F.
//!
//! Combines deterministic rule output with ONNX inference scores.
//! Rules are always the safety floor; ML adjusts confidence upward or downward.
//!
//! # Merge semantics
//!
//! ```text
//! Rule        ML score   Result
//! ─────────── ────────── ──────────────
//! Confirmed   ≥ 0.70     Confirmed      (ML agrees — no change needed)
//! Probable    ≥ 0.80     Confirmed      (ML upgrades)
//! Possible    ≥ 0.85     Probable       (ML upgrades)
//! Inconclusive≥ 0.80     Possible       (ML rescues borderline)
//! Confirmed   < 0.30     Probable       (ML disagrees — cautious downgrade)
//! any         otherwise  unchanged      (ML abstains or is noisy)
//! ```
//!
//! Temporal anomaly boost: if the MiniPatch-TST per-class score > 0.70 for a
//! class, the class confidence is boosted one level (capped at Confirmed).
//!
//! ML-only detections: when rules produced no diagnosis for a class but the
//! per-class ML score ≥ 0.90, a `Possible` diagnosis is injected with
//! `severity = Low`. These are never marked AutoSafe.

use std::collections::HashMap;

use inference::{InferenceResult, TelemetryAnomalyScore};
use trigger_engine::IssueClass;

use crate::types::{
    Confidence, DetectionMethod, Diagnosis, DiagnosisOutput, Evidence, MlContribution, Severity,
};

// ── Public API ────────────────────────────────────────────────────────────────

/// Merge ML inference scores into a rule-produced `DiagnosisOutput`.
///
/// `ml_scores`     — per-class ONNX scores (crash classifier, process anomaly)
/// `temporal_score` — optional MiniPatch-TST temporal anomaly output
///
/// Returns a new `DiagnosisOutput` with updated confidences and
/// `contributing_ml` fields populated. `detection_method` is updated to
/// `RulesPlusMl` or `MlOnly` as appropriate.
pub fn merge_ml(
    mut output: DiagnosisOutput,
    ml_scores: &HashMap<IssueClass, InferenceResult>,
    temporal_score: Option<&TelemetryAnomalyScore>,
) -> DiagnosisOutput {
    if ml_scores.is_empty() && temporal_score.is_none() {
        return output;
    }

    let mut ml_used = false;

    // ── Step 1: augment existing rule diagnoses ───────────────────────────────
    for diagnosis in &mut output.diagnoses {
        let class = &diagnosis.issue_class;

        // Per-class ONNX score (e.g. crash classifier, process anomaly)
        if let Some(ir) = ml_scores.get(class) {
            let old = diagnosis.confidence.clone();
            let new = apply_ml_confidence(&old, ir.confidence);

            diagnosis.contributing_ml = Some(MlContribution {
                model_name:        ir.model_name.clone(),
                model_version:     ir.model_version.clone(),
                ml_confidence:     ir.confidence,
                ml_class:          ir.predicted_class.clone(),
                merged_confidence: new.clone(),
            });

            if new != old {
                tracing::debug!(
                    class = ?class,
                    old = ?old,
                    new = ?new,
                    ml_conf = ir.confidence,
                    model = %ir.model_name,
                    "ML adjusted diagnosis confidence"
                );
            }
            diagnosis.confidence = new;

            if !output.ml_models_invoked.contains(&ir.model_name) {
                output.ml_models_invoked.push(ir.model_name.clone());
            }
            ml_used = true;
        }

        // Temporal anomaly boost (per-class score from MiniPatch-TST)
        if let Some(ts) = temporal_score {
            if let Some(score) = temporal_class_score(ts, class) {
                if score > 0.70 {
                    diagnosis.confidence = boost_one_level(&diagnosis.confidence);
                    ml_used = true;
                    tracing::debug!(
                        class = ?class,
                        temporal_score = score,
                        "Temporal anomaly boost applied"
                    );
                }
            }

            let model_name = "temporal_anomaly_v1".to_string();
            if !output.ml_models_invoked.contains(&model_name) {
                output.ml_models_invoked.push(model_name);
            }
        }
    }

    // ── Step 2: ML-only detections (rules produced nothing for this class) ────
    let existing_classes: Vec<IssueClass> = output
        .diagnoses
        .iter()
        .map(|d| d.issue_class.clone())
        .collect();

    for (class, ir) in ml_scores {
        if existing_classes.contains(class) {
            continue; // already handled in step 1
        }
        if ir.confidence < 0.90 {
            continue; // ML not confident enough to act alone
        }

        tracing::info!(
            class = ?class,
            ml_conf = ir.confidence,
            model = %ir.model_name,
            "ML-only detection: injecting Possible diagnosis"
        );

        let diagnosis = Diagnosis {
            issue_class:    class.clone(),
            confidence:     Confidence::Possible,
            severity:       Severity::Low,
            title:          format!("ML-detected anomaly: {:?}", class),
            explanation:    format!(
                "No deterministic rule fired, but the '{}' model (v{}) flagged this \
                 class with {:.0}% confidence. Review recommended before acting.",
                ir.model_name, ir.model_version, ir.confidence * 100.0,
            ),
            evidence:       vec![
                Evidence::field("ml_confidence", ir.confidence)
                    .with_context(&format!("predicted_class={}", ir.predicted_class)),
            ],
            contributing_ml: Some(MlContribution {
                model_name:        ir.model_name.clone(),
                model_version:     ir.model_version.clone(),
                ml_confidence:     ir.confidence,
                ml_class:          ir.predicted_class.clone(),
                merged_confidence: Confidence::Possible,
            }),
            rule_name: "ml_only".into(),
        };

        output.diagnoses.push(diagnosis);

        if !output.ml_models_invoked.contains(&ir.model_name) {
            output.ml_models_invoked.push(ir.model_name.clone());
        }
        ml_used = true;
    }

    // ── Step 3: update detection_method ──────────────────────────────────────
    if ml_used {
        let had_rule_diagnoses = output
            .diagnoses
            .iter()
            .any(|d| d.rule_name != "ml_only");

        output.detection_method = if had_rule_diagnoses {
            DetectionMethod::RulesPlusMl
        } else {
            DetectionMethod::MlOnly
        };
    }

    output
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Apply the confidence merge table.
fn apply_ml_confidence(rule: &Confidence, ml_conf: f32) -> Confidence {
    match rule {
        Confidence::Probable     if ml_conf >= 0.80 => Confidence::Confirmed,
        Confidence::Possible     if ml_conf >= 0.85 => Confidence::Probable,
        Confidence::Inconclusive if ml_conf >= 0.80 => Confidence::Possible,
        Confidence::Confirmed    if ml_conf <  0.30 => Confidence::Probable,
        _ => rule.clone(),
    }
}

/// Boost confidence by one level (capped at Confirmed).
fn boost_one_level(c: &Confidence) -> Confidence {
    match c {
        Confidence::Inconclusive => Confidence::Possible,
        Confidence::Possible     => Confidence::Probable,
        Confidence::Probable     => Confidence::Confirmed,
        Confidence::Confirmed    => Confidence::Confirmed,
    }
}

/// Extract the per-class anomaly score from a temporal model output.
/// Matches on the string representation of `IssueClass`.
fn temporal_class_score(ts: &TelemetryAnomalyScore, class: &IssueClass) -> Option<f32> {
    let target = issue_class_label(class);
    ts.per_class_scores
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(target))
        .map(|(_, score)| *score)
}

/// Canonical string label for each IssueClass (must match model output_classes).
fn issue_class_label(class: &IssueClass) -> &'static str {
    match class {
        IssueClass::HighCpu        => "HighCpu",
        IssueClass::HighMemory     => "HighMemory",
        IssueClass::LowDisk        => "LowDisk",
        IssueClass::DiskIo         => "DiskIo",
        IssueClass::StartupSlow    => "StartupSlow",
        IssueClass::AppCrash       => "AppCrash",
        IssueClass::NetworkDns     => "NetworkDns",
        IssueClass::ServiceDown    => "ServiceDown",
        IssueClass::UpdateIssue    => "UpdateIssue",
        IssueClass::BatteryThermal => "BatteryThermal",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_output(diagnoses: Vec<Diagnosis>) -> DiagnosisOutput {
        DiagnosisOutput {
            schema_version:    "1.0".into(),
            diagnosis_id:      Uuid::new_v4(),
            snapshot_id:       Uuid::new_v4(),
            diagnosed_at:      Utc::now(),
            diagnoses,
            detection_method:  DetectionMethod::RulesOnly,
            ml_models_invoked: Vec::new(),
            elapsed_ms:        0,
        }
    }

    fn make_diagnosis(class: IssueClass, confidence: Confidence) -> Diagnosis {
        Diagnosis {
            issue_class:    class,
            confidence,
            severity:       Severity::Medium,
            title:          "test".into(),
            explanation:    "test".into(),
            evidence:       vec![],
            contributing_ml: None,
            rule_name:      "test_rule".into(),
        }
    }

    fn make_ir(class_label: &str, confidence: f32) -> InferenceResult {
        InferenceResult {
            model_name:         "test_model".into(),
            model_version:      "0.1.0".into(),
            predicted_class:    class_label.into(),
            confidence,
            class_probabilities: vec![(class_label.into(), confidence)],
            elapsed_ms:         1,
        }
    }

    // ── apply_ml_confidence unit tests ────────────────────────────────────────

    #[test]
    fn probable_plus_high_ml_becomes_confirmed() {
        assert_eq!(
            apply_ml_confidence(&Confidence::Probable, 0.85),
            Confidence::Confirmed
        );
    }

    #[test]
    fn probable_plus_low_ml_stays_probable() {
        assert_eq!(
            apply_ml_confidence(&Confidence::Probable, 0.70),
            Confidence::Probable
        );
    }

    #[test]
    fn possible_plus_high_ml_becomes_probable() {
        assert_eq!(
            apply_ml_confidence(&Confidence::Possible, 0.90),
            Confidence::Probable
        );
    }

    #[test]
    fn inconclusive_plus_high_ml_becomes_possible() {
        assert_eq!(
            apply_ml_confidence(&Confidence::Inconclusive, 0.82),
            Confidence::Possible
        );
    }

    #[test]
    fn confirmed_plus_disagreeing_ml_downgrades_to_probable() {
        assert_eq!(
            apply_ml_confidence(&Confidence::Confirmed, 0.25),
            Confidence::Probable
        );
    }

    #[test]
    fn confirmed_plus_agreeing_ml_stays_confirmed() {
        assert_eq!(
            apply_ml_confidence(&Confidence::Confirmed, 0.80),
            Confidence::Confirmed
        );
    }

    // ── merge_ml integration tests ────────────────────────────────────────────

    #[test]
    fn no_ml_scores_returns_unchanged() {
        let d = make_diagnosis(IssueClass::HighCpu, Confidence::Probable);
        let out = make_output(vec![d]);
        let result = merge_ml(out.clone(), &HashMap::new(), None);
        assert!(matches!(result.detection_method, DetectionMethod::RulesOnly));
        assert_eq!(result.diagnoses[0].confidence, Confidence::Probable);
    }

    #[test]
    fn ml_upgrades_probable_to_confirmed() {
        let d = make_diagnosis(IssueClass::HighCpu, Confidence::Probable);
        let out = make_output(vec![d]);
        let mut scores = HashMap::new();
        scores.insert(IssueClass::HighCpu, make_ir("HighCpu", 0.91));

        let result = merge_ml(out, &scores, None);
        assert_eq!(result.diagnoses[0].confidence, Confidence::Confirmed);
        assert!(matches!(result.detection_method, DetectionMethod::RulesPlusMl));
        assert!(result.diagnoses[0].contributing_ml.is_some());
    }

    #[test]
    fn ml_downgrades_confirmed_when_disagreeing() {
        let d = make_diagnosis(IssueClass::AppCrash, Confidence::Confirmed);
        let out = make_output(vec![d]);
        let mut scores = HashMap::new();
        scores.insert(IssueClass::AppCrash, make_ir("AppCrash", 0.20));

        let result = merge_ml(out, &scores, None);
        assert_eq!(result.diagnoses[0].confidence, Confidence::Probable);
    }

    #[test]
    fn ml_only_detection_injected_above_threshold() {
        let out = make_output(vec![]); // no rule diagnoses
        let mut scores = HashMap::new();
        scores.insert(IssueClass::HighMemory, make_ir("HighMemory", 0.95));

        let result = merge_ml(out, &scores, None);
        assert_eq!(result.diagnoses.len(), 1);
        assert_eq!(result.diagnoses[0].confidence, Confidence::Possible);
        assert_eq!(result.diagnoses[0].severity, Severity::Low);
        assert_eq!(result.diagnoses[0].rule_name, "ml_only");
        assert!(matches!(result.detection_method, DetectionMethod::MlOnly));
    }

    #[test]
    fn ml_only_not_injected_below_threshold() {
        let out = make_output(vec![]);
        let mut scores = HashMap::new();
        scores.insert(IssueClass::HighMemory, make_ir("HighMemory", 0.85));

        let result = merge_ml(out, &scores, None);
        assert!(result.diagnoses.is_empty());
        // detection_method stays RulesOnly because ml_used=false
        assert!(matches!(result.detection_method, DetectionMethod::RulesOnly));
    }

    #[test]
    fn temporal_boost_upgrades_possible_to_probable() {
        let d = make_diagnosis(IssueClass::HighCpu, Confidence::Possible);
        let out = make_output(vec![d]);

        let ts = TelemetryAnomalyScore {
            anomaly_score:           0.75,
            per_class_scores:        vec![("HighCpu".into(), 0.80)],
            top_anomalous_features:  vec![],
            elapsed_ms:              2,
        };

        let result = merge_ml(out, &HashMap::new(), Some(&ts));
        assert_eq!(result.diagnoses[0].confidence, Confidence::Probable);
        assert!(matches!(result.detection_method, DetectionMethod::RulesPlusMl));
    }

    #[test]
    fn temporal_boost_below_threshold_no_change() {
        let d = make_diagnosis(IssueClass::HighCpu, Confidence::Possible);
        let out = make_output(vec![d]);

        let ts = TelemetryAnomalyScore {
            anomaly_score:           0.50,
            per_class_scores:        vec![("HighCpu".into(), 0.60)],
            top_anomalous_features:  vec![],
            elapsed_ms:              2,
        };

        let result = merge_ml(out, &HashMap::new(), Some(&ts));
        assert_eq!(result.diagnoses[0].confidence, Confidence::Possible);
    }

    #[test]
    fn ml_models_invoked_populated() {
        let d = make_diagnosis(IssueClass::HighCpu, Confidence::Probable);
        let out = make_output(vec![d]);
        let mut scores = HashMap::new();
        scores.insert(IssueClass::HighCpu, make_ir("HighCpu", 0.85));

        let result = merge_ml(out, &scores, None);
        assert!(result.ml_models_invoked.contains(&"test_model".to_string()));
    }
}
