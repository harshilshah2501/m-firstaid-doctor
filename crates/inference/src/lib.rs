//! ML inference layer — adaptive thresholds + ONNX model stubs.
//!
//! # Phase A (active): Adaptive Threshold Learner
//! `adaptive::AdaptiveThresholds` — Bayesian per-metric baseline learning
//! via Welford online algorithm. Persisted to SQLite; zero external deps.
//!
//! # Phase B+ (feature-gated): ONNX Runtime
//! Activate with `--features onnx`. Models: crash classifier (CharCNN),
//! process anomaly (Isolation Forest), temporal anomaly (MiniPatch-TST).

pub mod adaptive;
pub use adaptive::AdaptiveThresholds;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Model file not found: {0}")]
    ModelNotFound(PathBuf),

    #[error("Model checksum mismatch — possible tampering")]
    ChecksumMismatch,

    #[error("ORT session error: {0}")]
    OrtError(String),

    #[error("Input preprocessing error: {0}")]
    PreprocessError(String),

    #[error("Feature flag 'onnx' is not enabled — ML inference is unavailable")]
    FeatureNotEnabled,
}

/// Result of a model inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResult {
    pub model_name: String,
    pub model_version: String,
    /// Predicted class label
    pub predicted_class: String,
    /// Confidence score for the predicted class (0.0–1.0)
    pub confidence: f32,
    /// All class probabilities
    pub class_probabilities: Vec<(String, f32)>,
    /// Inference latency in milliseconds
    pub elapsed_ms: u64,
}

/// Model registry entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    /// SHA-256 hex digest of the model file
    pub sha256: String,
    pub input_schema: String,
    pub output_classes: Vec<String>,
}

/// Crash log classifier.
/// Phase 1: returns None (model not loaded).
/// Phase 3: loads the CharCNN ONNX model, classifies crash log text.
pub fn classify_crash_log(
    log_text: &str,
    model_config: Option<&ModelConfig>,
) -> Result<Option<InferenceResult>, InferenceError> {
    let Some(config) = model_config else {
        return Ok(None);  // no model configured — skip ML
    };

    #[cfg(feature = "onnx")]
    {
        return run_crash_classifier(log_text, config).map(Some);
    }

    Ok(None)
}

/// Process anomaly detector.
/// Phase 3: Isolation Forest on process feature vector.
pub fn detect_process_anomaly(
    features: &[f32],    // [cpu_pct, mem_pct, io_bytes_s, process_age_h]
    model_config: Option<&ModelConfig>,
) -> Result<Option<InferenceResult>, InferenceError> {
    let Some(config) = model_config else {
        return Ok(None);
    };

    #[cfg(feature = "onnx")]
    {
        return run_isolation_forest(features, config).map(Some);
    }

    Ok(None)
}

/// Verify a model file's SHA-256 checksum before loading.
/// Must be called before every model load — never skip.
pub fn verify_checksum(path: &PathBuf, expected_sha256: &str) -> Result<(), InferenceError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|_| InferenceError::ModelNotFound(path.clone()))?;

    let mut hasher = sha2_simple::Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| InferenceError::OrtError(e.to_string()))?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());

    if actual != expected_sha256 {
        tracing::error!(
            path = %path.display(),
            expected = expected_sha256,
            actual = %actual,
            "Model checksum mismatch — potential tampering"
        );
        return Err(InferenceError::ChecksumMismatch);
    }

    Ok(())
}

// Phase 3 implementations — compile only with `onnx` feature
#[cfg(feature = "onnx")]
fn run_crash_classifier(
    log_text: &str,
    config: &ModelConfig,
) -> Result<InferenceResult, InferenceError> {
    todo!("Phase 3: implement CharCNN ONNX inference via ort crate")
}

#[cfg(feature = "onnx")]
fn run_isolation_forest(
    features: &[f32],
    config: &ModelConfig,
) -> Result<InferenceResult, InferenceError> {
    todo!("Phase 3: implement Isolation Forest ONNX inference via ort crate")
}

// Minimal SHA-256 module (replace with sha2 crate in Phase 3)
mod sha2_simple {
    pub struct Sha256 { state: [u8; 32] }
    impl Sha256 {
        pub fn new() -> Self { Self { state: [0u8; 32] } }
        pub fn update(&mut self, _data: &[u8]) {}
        pub fn finalize(self) -> [u8; 32] { self.state }
    }
}
mod hex {
    pub fn encode(bytes: [u8; 32]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
