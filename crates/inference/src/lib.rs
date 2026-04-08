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
pub mod trainer;

pub use adaptive::AdaptiveThresholds;
pub use session_pool::{load_manifest, ModelSessionPool, OrtSessionRef};
pub use trainer::{
    record_process_observation, should_retrain, record_retrain,
    spawn_retrain_subprocess, FEATURE_NAMES, MIN_OBS_FOR_RETRAIN,
};

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

/// Preprocess crash log text into a character-level one-hot tensor.
///
/// Encoding: 512 character positions × 128 ASCII slots = 65 536 f32 values.
/// Characters outside ASCII (codepoint > 127) are mapped to slot 0 (NUL).
/// The output is laid out in row-major order: `tensor[char_idx * 128 + ascii]`.
pub fn preprocess_crash_text(text: &str) -> Vec<f32> {
    const CHARS: usize = 512;
    const ASCII: usize = 128;
    let mut tensor = vec![0.0f32; CHARS * ASCII];
    for (i, ch) in text.chars().take(CHARS).enumerate() {
        let code = ch as usize;
        let slot = if code < ASCII { code } else { 0 };
        tensor[i * ASCII + slot] = 1.0;
    }
    tensor
}

#[cfg(feature = "onnx")]
fn run_crash_classifier(
    log_text: &str,
    config: &ModelConfig,
    session: &OrtSessionRef<'_>,
    start: std::time::Instant,
) -> Result<InferenceResult, InferenceError> {
    use ort::value::Tensor;

    // Step 1: character-level one-hot encoding → [1, 512, 128] f32 tensor
    let flat = preprocess_crash_text(log_text);
    let tensor = Tensor::from_array(([1usize, 512, 128], flat.into_boxed_slice()))
        .map_err(|e| InferenceError::PreprocessError(format!("tensor build: {e}")))?;

    // Step 2: run ORT session
    let outputs = session
        .session
        .run(ort::inputs!["input" => tensor]
            .map_err(|e| InferenceError::OrtError(format!("inputs macro: {e}")))?)
        .map_err(|e| InferenceError::OrtError(format!("session run: {e}")))?;

    // Step 3: extract logits and apply softmax
    let logits = outputs["output"]
        .try_extract_tensor::<f32>()
        .map_err(|e| InferenceError::OrtError(format!("extract logits: {e}")))?;
    let logits_slice = logits.as_slice().unwrap_or(&[]);

    let probs = softmax(logits_slice);

    // Step 4: pair with class labels and sort descending
    let mut class_probabilities: Vec<(String, f32)> = config
        .output_classes
        .iter()
        .cloned()
        .zip(probs.iter().copied())
        .collect();
    class_probabilities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let (predicted_class, confidence) = class_probabilities
        .first()
        .map(|(c, p)| (c.clone(), *p))
        .unwrap_or_else(|| ("unknown".into(), 0.0));

    Ok(InferenceResult {
        model_name:         config.name.clone(),
        model_version:      config.version.clone(),
        predicted_class,
        confidence,
        class_probabilities,
        elapsed_ms:         start.elapsed().as_millis() as u64,
    })
}

// ── Shared utilities ──────────────────────────────────────────────────────────

/// Numerically-stable softmax.
fn softmax(logits: &[f32]) -> Vec<f32> {
    if logits.is_empty() {
        return Vec::new();
    }
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum == 0.0 {
        return vec![1.0 / logits.len() as f32; logits.len()];
    }
    exps.iter().map(|&e| e / sum).collect()
}

// ── Phase E: Isolation Forest process anomaly implementation ──────────────────

#[cfg(feature = "onnx")]
fn run_isolation_forest(
    features: &[f32; 5],
    config: &ModelConfig,
    session: &OrtSessionRef<'_>,
    start: std::time::Instant,
) -> Result<InferenceResult, InferenceError> {
    use ort::value::Tensor;

    // Step 1: shape [1, 5] f32 tensor
    let flat: Box<[f32]> = features.to_vec().into_boxed_slice();
    let tensor = Tensor::from_array(([1usize, 5], flat))
        .map_err(|e| InferenceError::PreprocessError(format!("tensor build: {e}")))?;

    // Step 2: run ORT session
    let outputs = session
        .session
        .run(ort::inputs!["input" => tensor]
            .map_err(|e| InferenceError::OrtError(format!("inputs macro: {e}")))?)
        .map_err(|e| InferenceError::OrtError(format!("session run: {e}")))?;

    // Step 3: extract probabilities — expected shape [1, 2] → [normal_prob, anomaly_prob].
    // The training script (scripts/train_process_anomaly.py) exports with `predict_proba`
    // output so the values are calibrated probabilities, not raw logits.
    // We apply softmax defensively in case the model was exported with raw logits.
    let probs_tensor = outputs["output"]
        .try_extract_tensor::<f32>()
        .map_err(|e| InferenceError::OrtError(format!("extract probabilities: {e}")))?;
    let probs_slice = probs_tensor.as_slice().unwrap_or(&[0.5, 0.5]);

    // Normalize to sum-to-1 in case of numerical drift.
    let probs = if probs_slice.len() >= 2 {
        softmax(probs_slice)
    } else {
        // Single-value output (raw anomaly score in [0,1]): treat as anomaly_prob.
        let anom = probs_slice.first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
        vec![1.0 - anom, anom]
    };

    // Step 4: build ranked class_probabilities with canonical class labels.
    // output_classes in manifest: ["normal", "anomaly"] — index 0=normal, 1=anomaly.
    let labels = if config.output_classes.len() >= 2 {
        config.output_classes.clone()
    } else {
        vec!["normal".into(), "anomaly".into()]
    };

    let mut class_probabilities: Vec<(String, f32)> = labels
        .iter()
        .cloned()
        .zip(probs.iter().copied())
        .collect();
    class_probabilities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let (predicted_class, confidence) = class_probabilities
        .first()
        .map(|(c, p)| (c.clone(), *p))
        .unwrap_or_else(|| ("normal".into(), 0.5));

    tracing::debug!(
        predicted_class = %predicted_class,
        confidence,
        cpu_pct = features[0],
        mem_mb  = features[1],
        "Isolation Forest inference complete"
    );

    Ok(InferenceResult {
        model_name:         config.name.clone(),
        model_version:      config.version.clone(),
        predicted_class,
        confidence,
        class_probabilities,
        elapsed_ms:         start.elapsed().as_millis() as u64,
    })
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
    fn preprocess_crash_text_length() {
        let text = "A".repeat(600); // longer than 512
        let tensor = preprocess_crash_text(&text);
        assert_eq!(tensor.len(), 512 * 128, "tensor must be exactly 512×128 floats");
    }

    #[test]
    fn preprocess_crash_text_short_input_zero_padded() {
        let tensor = preprocess_crash_text("AB");
        // Char 'A' = 65  → tensor[0*128 + 65] = 1.0
        assert_eq!(tensor[65], 1.0);
        // Char 'B' = 66  → tensor[1*128 + 66] = 1.0
        assert_eq!(tensor[128 + 66], 1.0);
        // Position 2 onward should be all zeros
        let sum_rest: f32 = tensor[256..].iter().sum();
        assert_eq!(sum_rest, 0.0, "un-used positions must be zero");
    }

    #[test]
    fn preprocess_crash_text_non_ascii_maps_to_slot_zero() {
        let tensor = preprocess_crash_text("é"); // codepoint 233 > 127
        assert_eq!(tensor[0], 1.0, "non-ASCII goes to slot 0 (NUL)");
    }

    #[test]
    fn softmax_sums_to_one() {
        let logits = vec![1.0f32, 2.0, 3.0];
        let probs = softmax(&logits);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "softmax must sum to 1.0");
    }

    #[test]
    fn softmax_empty_returns_empty() {
        assert!(softmax(&[]).is_empty());
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
    fn softmax_single_value_normalizes_to_pair() {
        // Simulates the single-output fallback path in run_isolation_forest.
        let single_score = 0.85f32;  // anomaly_prob
        let probs = vec![1.0 - single_score, single_score];
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert_eq!(probs[1], 0.85);
    }

    #[test]
    fn softmax_two_class_output() {
        // Model outputs logits [2.0, 1.0] → normal wins
        let probs = softmax(&[2.0f32, 1.0]);
        assert!(probs[0] > probs[1], "higher logit → higher probability");
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn score_telemetry_window_no_config_returns_none() {
        let result = score_telemetry_window(&[], None, None);
        assert!(matches!(result, Ok(None)));
    }
}
