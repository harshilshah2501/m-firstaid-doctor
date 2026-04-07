use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trigger_engine::IssueClass;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisOutput {
    pub schema_version: String,
    pub diagnosis_id: Uuid,
    pub snapshot_id: Uuid,
    pub diagnosed_at: DateTime<Utc>,
    pub diagnoses: Vec<Diagnosis>,
    pub detection_method: DetectionMethod,
    pub ml_models_invoked: Vec<String>,
    pub elapsed_ms: u64,
}

impl DiagnosisOutput {
    pub fn is_empty(&self) -> bool {
        self.diagnoses.is_empty()
    }

    pub fn max_severity(&self) -> Option<&Severity> {
        self.diagnoses.iter().map(|d| &d.severity).max_by_key(|s| *s as *const _)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub issue_class: IssueClass,
    pub confidence: Confidence,
    pub severity: Severity,
    pub title: String,
    pub explanation: String,
    pub evidence: Vec<Evidence>,
    pub contributing_ml: Option<MlContribution>,
    pub rule_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub field: String,
    pub value: serde_json::Value,
    pub threshold: Option<serde_json::Value>,
    pub context: Option<String>,
}

impl Evidence {
    pub fn field(field: &str, value: impl Serialize) -> Self {
        Self {
            field: field.into(),
            value: serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
            threshold: None,
            context: None,
        }
    }

    pub fn with_threshold(mut self, threshold: impl Serialize) -> Self {
        self.threshold = Some(serde_json::to_value(threshold).unwrap_or(serde_json::Value::Null));
        self
    }

    pub fn with_context(mut self, ctx: &str) -> Self {
        self.context = Some(ctx.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MlContribution {
    pub model_name: String,
    pub model_version: String,
    pub ml_confidence: f32,
    pub ml_class: String,
    pub merged_confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Inconclusive,
    Possible,
    Probable,
    Confirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DetectionMethod {
    RulesOnly,
    RulesPlusMl,
    MlOnly,
}
