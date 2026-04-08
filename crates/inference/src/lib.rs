//! ML inference layer for m-firstaid-doctor.
//!
//! # Active (Phase A)
//! - [`AdaptiveThresholds`] — Bayesian per-metric baseline learning via Welford
//!   online algorithm. Pure Rust, zero external ML deps, persisted to SQLite.
//!
//! # Feature-gated (Phase B+, `--features onnx`)
//! - [`ModelSessionPool`] — lazy-loading ONNX session pool with idle eviction.
//! - [`verify_checksum`] — SHA-256 integrity gate before every model load.
//! - [`classify_crash_log`], [`detect_process_anomaly`] — model stubs activated
//!   by Phase D/E.

pub mod adaptive;
pub mod session_pool;

pub use adaptive::AdaptiveThresholds;
pub use session_pool::{load_manifest, ModelSessionPool, OrtSessionRef};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;
use thiserror::Error;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Model file not found: {0}")]
    ModelNotFound(PathBuf),

    #[error("Model checksum mismatch — possible tampering or corruption")]
    ChecksumMismatch,

    #[error("ORT session error: {0}")]
    OrtError(String),

    #[error("Input preprocessing error: {0}")]
    PreprocessError(String),

    #[error("Feature flag 'onnx' is not enabled — ML inference is unavailable")]
    FeatureNotEnabled,
}

// ── Core types ────────────────────────────────────────────────────────────────

/// Result of a model inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResult {
    pub model_name: String,
    pub model_version: String,
    /// Predicted class label
    pub predicted_class: String,
    /// Confidence score for the predicted class (0.0–1.0)
    pub confidence: f32,
    /// All class probabilities, sorted descending by probability
    pub class_probabilities: Vec<(String, f32)>,
    /// Inference latency in milliseconds
    pub elapsed_ms: u64,
}

/// Manifest entry for a single ONNX model.
/// Loaded from `models/manifest.json` at agent startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Unique model name (e.g. "crash_classifier_v1")
    pub name: String,
    /// Semantic version string
    pub version: String,
    /// Path to the `.onnx` file (relative to the agent binary or absolute)
    pub path: PathBuf,
    /// SHA-256 hex digest — verified before every load
    pub sha256: String,
    /// Human-readable input tensor spec (e.g. "char_onehot_[512x128]")
    pub input_schema: String,
    /// Output class labels in the same order as the model's output logits
    pub output_classes: Vec<String>,
}

// ── Checksum verification (safety gate) ───────────────────────────────────────

/// Verify a model file's SHA-256 checksum before loading.
///
/// **Must be called before every ORT session creation.** The session pool
/// calls this automatically — do not bypass.
///
/// # Errors
/// - [`InferenceError::ModelNotFound`] if the file cannot be opened.
/// - [`InferenceError::ChecksumMismatch`] if the digest doesn't match.
pub fn verify_checksum(path: &PathBuf, expected_sha256: &str) -> Result<(), InferenceError> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| InferenceError::ModelNotFound(path.clone()))?;

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = file.read(&mut buf)
            .map_err(|e| InferenceError::OrtError(format!("read '{}': {e}", path.display())))?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());

    if actual != expected_sha256.to_ascii_lowercase() {
        tracing::error!(
            path = %path.display(),
            expected = expected_sha256,
            actual = %actual,
            "SHA-256 mismatch — refusing to load model"
        );
        return Err(InferenceError::ChecksumMismatch);
    }

    tracing::debug!(model_path = %path.display(), "Checksum verified");
    Ok(())
}

// ── High-level inference functions ────────────────────────────────────────────
// These are the public API for the detector rules (Phase D/E/C).
// Each returns Ok(None) when ML is unavailable — callers fall back to rules.

/// Crash log text classifier (CharCNN).
///
/// Phase D: classifies raw crash log text into an IssueClass + probable cause.
/// Returns `Ok(None)` when the model is not configured or `onnx` is disabled.
pub fn classify_crash_log(
    log_text: &str,
    model_config: Option<&ModelConfig>,
    pool: Option<&ModelSessionPool>,
) -> Result<Option<InferenceResult>, InferenceError> {
    let (Some(config), Some(pool)) = (model_config, pool) else {
        return Ok(None);
    };

    #[cfg(feature = "onnx")]
    {
        let start = std::time::Instant::now();
        return pool.with_session(config, |session| {
            run_crash_classifier(log_text, config, session, start)
        }).map(Some);
    }

    #[allow(unreachable_code)]
    Ok(None)
}

/// Process behavior anomaly scorer (Isolation Forest).
///
/// Phase E: scores how anomalous a process's resource usage is relative to
/// its historical baseline on this machine.
/// Returns `Ok(None)` when the model is not configured or `onnx` is disabled.
/// Feature vector layout: `[cpu_pct, mem_mb, io_bytes_per_sec, process_age_hours, cpu_trend_5m]`
pub fn detect_process_anomaly(
    features: &[f32; 5],
    model_config: Option<&ModelConfig>,
    pool: Option<&ModelSessionPool>,
) -> Result<Option<InferenceResult>, InferenceError> {
    let (Some(config), Some(pool)) = (model_config, pool) else {
        return Ok(None);
    };

    #[cfg(feature = "onnx")]
    {
        let start = std::time::Instant::now();
        return pool.with_session(config, |session| {
            run_isolation_forest(features, config, session, start)
        }).map(Some);
    }

    #[allow(unreachable_code)]
    Ok(None)
}

/// Multi-variate temporal anomaly scorer (MiniPatch-TST Transformer).
///
/// Phase C: scores a 30-step window of 52-feature FeatureSet time-series.
/// Returns `Ok(None)` when the model is not configured or `onnx` is disabled.
/// Feature time-series window: up to 30 FeatureSet entries (oldest first), each 52 floats.
pub fn score_telemetry_window(
    history_features: &[Vec<f32>],
    model_config: Option<&ModelConfig>,
    pool: Option<&ModelSessionPool>,
) -> Result<Option<TelemetryAnomalyScore>, InferenceError> {
    let (Some(config), Some(pool)) = (model_config, pool) else {
        return Ok(None);
    };
    let _ = (history_features, pool, config);  // suppress warnings until Phase C

    #[cfg(feature = "onnx")]
    {
        todo!("Phase C: implement MiniPatch-TST temporal anomaly scorer")
    }

    #[allow(unreachable_code)]
    Ok(None)
}

/// Output of the temporal anomaly transformer (Phase C).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryAnomalyScore {
    /// Overall anomaly score for this time window (0.0 = normal, 1.0 = highly anomalous)
    pub anomaly_score: f32,
    /// Per-issue-class anomaly contribution (issue_class_name, score)
    pub per_class_scores: Vec<(String, f32)>,
    /// Top features contributing most to the anomaly signal
    pub top_anomalous_features: Vec<(String, f32)>,
    /// Inference latency in milliseconds
    pub elapsed_ms: u64,
}

// ── Phase D: CharCNN crash classifier implementation ───────────────────────────

#[cfg(feature = "onnx")]
fn run_crash_classifier(
    log_text: &str,
    config: &ModelConfig,
    _session: &OrtSessionRef<'_>,
    start: std::time::Instant,
) -> Result<InferenceResult, InferenceError> {
    // TODO Phase D: implement CharCNN preprocessing and ORT inference
    // Steps:
    //   1. Truncate log_text to first 512 chars
    //   2. One-hot encode each char → [512, 128] f32 tensor
    //   3. session.run(ort::inputs!["input" => tensor]?)
    //   4. Softmax outputs → class_probabilities
    //   5. Return InferenceResult with top class + confidence
    todo!("Phase D: implement CharCNN crash classifier ORT inference")
}

// ── Phase E: Isolation Forest process anomaly implementation ──────────────────

#[cfg(feature = "onnx")]
fn run_isolation_forest(
    features: &[f32; 5],
    config: &ModelConfig,
    _session: &OrtSessionRef<'_>,
    start: std::time::Instant,
) -> Result<InferenceResult, InferenceError> {
    // TODO Phase E: implement Isolation Forest ORT inference
    // Steps:
    //   1. Shape features as [1, 5] f32 tensor
    //   2. session.run(ort::inputs!["input" => tensor]?)
    //   3. Output: [anomaly_score] f32 (0=normal, 1=anomaly)
    //   4. Return InferenceResult with predicted_class = "normal"|"anomaly"
    todo!("Phase E: implement Isolation Forest ORT inference")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_checksum_file_not_found() {
        let result = verify_checksum(&PathBuf::from("/does/not/exist.onnx"), "abc");
        assert!(matches!(result, Err(InferenceError::ModelNotFound(_))));
    }

    #[test]
    fn verify_checksum_mismatch_detected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello world").unwrap();
        let result = verify_checksum(
            &tmp.path().to_path_buf(),
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(matches!(result, Err(InferenceError::ChecksumMismatch)));
    }

    #[test]
    fn verify_checksum_correct_passes() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello world").unwrap();
        // SHA-256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe04294e576cf55ab8a7c7e8a4f  ← wait that's SHA-1
        // Correct SHA-256("hello world"):
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe04294e576cf55ab8a7c7e8a4f";
        // Actually let's compute it properly
        let mut hasher = Sha256::new();
        hasher.update(b"hello world");
        let actual = hex::encode(hasher.finalize());
        // Use the actual computed value so the test is self-consistent
        let result = verify_checksum(&tmp.path().to_path_buf(), &actual);
        assert!(result.is_ok(), "correct checksum should pass: {result:?}");
    }

    #[test]
    fn classify_crash_log_no_config_returns_none() {
        let result = classify_crash_log("Exception in thread main", None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn detect_process_anomaly_no_config_returns_none() {
        let result = detect_process_anomaly(&[80.0, 512.0, 1024.0, 2.5, 5.0], None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn score_telemetry_window_no_config_returns_none() {
        let result = score_telemetry_window(&[], None, None);
        assert!(matches!(result, Ok(None)));
    }
}
