//! Temporal window preprocessing for the MiniPatch-TST anomaly transformer.
//!
//! # Architecture summary
//! Input : `[1, T_MAX, N_FEATURES]` f32 — zero-padded history (oldest first)
//! Model : Patch embedding (patch=5) → 4× TransformerEncoder (d=64, h=4) →
//!         GlobalAvgPool → per-class sigmoid scores + global max
//! Output: `[1, N_CLASSES]` f32 — one anomaly probability per `IssueClass`
//!
//! # Padding convention
//! If the caller supplies fewer than `T_MAX` steps the *oldest* positions are
//! zero-filled so recent data always occupies the *trailing* positions.
//! A zero-padded row is indistinguishable from "system off" state, which the
//! model is trained to score as normal — harmless.
//!
//! # Feature names
//! `N_FEATURES` and ordering must match `feature_extractor::TEMPORAL_FEATURE_NAMES`
//! and the Python training script column list exactly.

// Re-export constants so callers don't need feature-extractor as a dep.
pub use feature_extractor::N_TEMPORAL_FEATURES;
pub use feature_extractor::TEMPORAL_FEATURE_NAMES;

/// Maximum history steps fed to the transformer.
pub const T_MAX: usize = 30;

/// Minimum steps before temporal inference is attempted.
/// Below this threshold the window is too short to carry meaningful trend signal.
pub const MIN_HISTORY_STEPS: usize = 5;

/// Canonical IssueClass output order — must match `models/manifest.json`
/// `output_classes` for `temporal_anomaly_v1` and the training script.
pub const ISSUE_CLASS_ORDER: &[&str] = &[
    "HighCpu",
    "HighMemory",
    "LowDisk",
    "DiskIo",
    "StartupSlow",
    "AppCrash",
    "NetworkDns",
    "ServiceDown",
    "UpdateIssue",
    "BatteryThermal",
];

/// Number of IssueClasses the temporal model scores.
pub const N_CLASSES: usize = ISSUE_CLASS_ORDER.len();  // 10

// ── Preprocessing ─────────────────────────────────────────────────────────────

/// Flatten a history window into a `[T_MAX * N_FEATURES]` f32 buffer.
///
/// `history[i]` = feature vector for time step `i`, **oldest first**.
/// Each vector must have exactly `N_FEATURES` elements.
///
/// Rows are zero-padded at the *front* if `history.len() < T_MAX`.
/// If a row has fewer than `N_FEATURES` elements, missing slots are zero.
pub fn preprocess_window(history: &[Vec<f32>]) -> Vec<f32> {
    let mut out = vec![0.0f32; T_MAX * N_TEMPORAL_FEATURES];
    let n = history.len().min(T_MAX);
    let offset = T_MAX - n;   // zero-pad oldest positions
    for (i, row) in history.iter().rev().take(n).enumerate() {
        // Fill newest-first into the trailing rows
        let t = T_MAX - 1 - i;
        let base = t * N_TEMPORAL_FEATURES;
        let copy_len = row.len().min(N_TEMPORAL_FEATURES);
        out[base..base + copy_len].copy_from_slice(&row[..copy_len]);
    }
    let _ = offset; // explicit: first `offset` rows stay zero
    out
}

// ── Top anomalous features ────────────────────────────────────────────────────

/// Heuristic: identify which features vary most across the window.
///
/// Returns the top `k` feature names paired with their coefficient of
/// variation (std / |mean| + ε), descending. High CoV → strong trend or
/// oscillation → contributes most to the anomaly signal.
pub fn top_anomalous_features(history: &[Vec<f32>], k: usize) -> Vec<(String, f32)> {
    if history.is_empty() || k == 0 {
        return Vec::new();
    }

    let n = history.len() as f64;
    let mut scores: Vec<(usize, f32)> = (0..N_TEMPORAL_FEATURES)
        .filter_map(|fi| {
            let values: Vec<f64> = history
                .iter()
                .map(|row| row.get(fi).copied().unwrap_or(0.0) as f64)
                .collect();

            let mean = values.iter().sum::<f64>() / n;
            let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            let std = variance.sqrt();

            // Coefficient of variation (higher = more anomalous)
            let cov = (std / (mean.abs() + 1e-6)) as f32;
            if cov.is_finite() && cov > 0.0 {
                Some((fi, cov))
            } else {
                None
            }
        })
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores
        .into_iter()
        .take(k)
        .map(|(fi, score)| (TEMPORAL_FEATURE_NAMES[fi].to_string(), score))
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn zeros() -> Vec<f32> {
        vec![0.0f32; N_TEMPORAL_FEATURES]
    }

    fn const_row(v: f32) -> Vec<f32> {
        vec![v; N_TEMPORAL_FEATURES]
    }

    #[test]
    fn preprocess_exact_window_no_padding() {
        let history: Vec<Vec<f32>> = (0..T_MAX).map(|_| zeros()).collect();
        let flat = preprocess_window(&history);
        assert_eq!(flat.len(), T_MAX * N_TEMPORAL_FEATURES);
        assert!(flat.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn preprocess_short_window_pads_front() {
        // Only 2 steps provided — the last 2 rows of the output should be filled,
        // the first T_MAX-2 rows should be zero.
        let history = vec![const_row(1.0), const_row(2.0)];
        let flat = preprocess_window(&history);
        assert_eq!(flat.len(), T_MAX * N_TEMPORAL_FEATURES);

        // Oldest filled row = T_MAX-2 (index 28 for T_MAX=30)
        let row_28_start = (T_MAX - 2) * N_TEMPORAL_FEATURES;
        assert_eq!(flat[row_28_start], 1.0, "oldest step → second-to-last row");

        // Newest filled row = T_MAX-1
        let row_29_start = (T_MAX - 1) * N_TEMPORAL_FEATURES;
        assert_eq!(flat[row_29_start], 2.0, "newest step → last row");

        // All rows before T_MAX-2 should be zero
        let prefix: f32 = flat[..(T_MAX - 2) * N_TEMPORAL_FEATURES].iter().sum();
        assert_eq!(prefix, 0.0, "prefix rows must be zero-padded");
    }

    #[test]
    fn preprocess_longer_than_t_max_truncates_oldest() {
        // 35 steps — only the last T_MAX=30 should survive.
        let history: Vec<Vec<f32>> = (0u32..35)
            .map(|i| vec![i as f32; N_TEMPORAL_FEATURES])
            .collect();
        let flat = preprocess_window(&history);
        // Newest step (index 34) should be at row T_MAX-1
        let last_row_start = (T_MAX - 1) * N_TEMPORAL_FEATURES;
        assert_eq!(flat[last_row_start], 34.0);
        // Second-newest (index 33) at row T_MAX-2
        let second_last_start = (T_MAX - 2) * N_TEMPORAL_FEATURES;
        assert_eq!(flat[second_last_start], 33.0);
    }

    #[test]
    fn top_anomalous_features_empty_history() {
        assert!(top_anomalous_features(&[], 5).is_empty());
    }

    #[test]
    fn top_anomalous_features_constant_history_no_variance() {
        let history: Vec<Vec<f32>> = (0..10).map(|_| const_row(50.0)).collect();
        // Constant values → CoV=0 → all filtered → empty result
        let top = top_anomalous_features(&history, 5);
        assert!(top.is_empty(), "constant signal has no anomalous features");
    }

    #[test]
    fn top_anomalous_features_finds_varying_feature() {
        // Feature 0 (cpu_usage_avg_1m) oscillates 0↔100; others are constant.
        let mut history: Vec<Vec<f32>> = (0..10).map(|_| zeros()).collect();
        for (i, row) in history.iter_mut().enumerate() {
            row[0] = if i % 2 == 0 { 0.0 } else { 100.0 };
        }
        let top = top_anomalous_features(&history, 3);
        assert!(!top.is_empty());
        assert_eq!(top[0].0, "cpu_usage_avg_1m", "most variable feature must be first");
    }

    #[test]
    fn issue_class_order_has_correct_count() {
        assert_eq!(ISSUE_CLASS_ORDER.len(), N_CLASSES);
        assert_eq!(N_CLASSES, 10);
    }
}
