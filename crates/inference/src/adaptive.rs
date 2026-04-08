//! Adaptive threshold learner using the Welford online algorithm.
//!
//! Learns machine-specific "normal" baselines for each metric from accumulated
//! snapshot history. Replaces hard-coded trigger thresholds with personalized
//! ones (mean + k·σ) once enough samples have been collected.
//!
//! # Algorithm
//! Welford's online algorithm computes running mean and variance in a single
//! pass with O(1) memory per metric — no history window needed.
//!
//! # Personalization guarantee
//! Until `MIN_SAMPLES` have been observed, `threshold()` returns `None` and
//! the caller falls back to the hard-coded rule default. This ensures the ML
//! layer never degrades accuracy in the cold-start period.

use rusqlite::{Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// Minimum samples required before a learned threshold is trusted.
const MIN_SAMPLES: u64 = 50;

/// Welford online mean/variance state for a single metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WelfordState {
    /// Number of samples seen so far.
    n: u64,
    /// Running mean (Welford M).
    mean: f64,
    /// Running sum of squared deviations (Welford M2).
    m2: f64,
}

impl WelfordState {
    fn new() -> Self {
        Self { n: 0, mean: 0.0, m2: 0.0 }
    }

    /// Incorporate a new observation using Welford's online update.
    fn update(&mut self, value: f64) {
        self.n += 1;
        let delta = value - self.mean;
        self.mean += delta / self.n as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;
    }

    /// Population variance. Returns 0.0 if fewer than 2 samples.
    fn variance(&self) -> f64 {
        if self.n < 2 { 0.0 } else { self.m2 / self.n as f64 }
    }

    /// Standard deviation.
    fn stddev(&self) -> f64 {
        self.variance().sqrt()
    }
}

/// Per-metric adaptive thresholds learned from local machine history.
///
/// # Usage
/// ```rust,ignore
/// let mut at = AdaptiveThresholds::new();
/// // Feed historical observations:
/// at.update("cpu_usage_avg_5m", 42.0);
/// // After MIN_SAMPLES, get a personalized threshold:
/// let thr = at.threshold("cpu_usage_avg_5m", 2.5); // mean + 2.5σ
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveThresholds {
    stats: HashMap<String, WelfordState>,
}

impl AdaptiveThresholds {
    pub fn new() -> Self {
        Self { stats: HashMap::new() }
    }

    /// Record a new observation for `metric`. Call once per collection cycle.
    pub fn update(&mut self, metric: &str, value: f64) {
        self.stats
            .entry(metric.to_string())
            .or_insert_with(WelfordState::new)
            .update(value);
    }

    /// Batch-update from a slice of (metric, value) pairs. Convenience helper.
    pub fn update_all(&mut self, observations: &[(&str, f64)]) {
        for (metric, value) in observations {
            self.update(metric, *value);
        }
    }

    /// Returns the learned threshold `mean + sigma * stddev` for `metric`.
    ///
    /// Returns `None` if fewer than `MIN_SAMPLES` have been observed —
    /// the caller must fall back to the hard-coded rule default.
    ///
    /// `sigma` controls sensitivity:
    /// - 2.0 → flags top ~2.3% of observed values (aggressive)
    /// - 2.5 → flags top ~0.6% (balanced, recommended for most metrics)
    /// - 3.0 → flags top ~0.13% (conservative, for noisy metrics)
    pub fn threshold(&self, metric: &str, sigma: f64) -> Option<f64> {
        let state = self.stats.get(metric)?;
        if state.n < MIN_SAMPLES {
            debug!(
                metric,
                n = state.n,
                required = MIN_SAMPLES,
                "Insufficient samples — using rule default"
            );
            return None;
        }
        let threshold = state.mean + sigma * state.stddev();
        debug!(
            metric,
            mean = state.mean,
            stddev = state.stddev(),
            sigma,
            threshold,
            "Adaptive threshold computed"
        );
        Some(threshold)
    }

    /// Current sample count for a metric. Useful for diagnostics / UI.
    pub fn sample_count(&self, metric: &str) -> u64 {
        self.stats.get(metric).map(|s| s.n).unwrap_or(0)
    }

    /// Returns true if a metric has accumulated enough samples to be trusted.
    pub fn is_calibrated(&self, metric: &str) -> bool {
        self.sample_count(metric) >= MIN_SAMPLES
    }

    /// Overall calibration status: fraction of known metrics that are calibrated.
    pub fn calibration_fraction(&self) -> f64 {
        if self.stats.is_empty() { return 0.0; }
        let calibrated = self.stats.values().filter(|s| s.n >= MIN_SAMPLES).count();
        calibrated as f64 / self.stats.len() as f64
    }

    // ── Persistence ──────────────────────────────────────────────────────────

    /// Persist learned parameters to the `model_params` SQLite table.
    /// Call after each `update()` batch (once per collection cycle).
    pub fn save(&self, conn: &Connection) -> SqlResult<()> {
        let json = serde_json::to_string(self)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute(
            "INSERT INTO model_params (param_key, param_json, updated_at)
             VALUES ('adaptive_thresholds', ?1, datetime('now'))
             ON CONFLICT(param_key) DO UPDATE SET
                 param_json = excluded.param_json,
                 updated_at = excluded.updated_at",
            rusqlite::params![json],
        )?;
        debug!("AdaptiveThresholds persisted ({} metrics)", self.stats.len());
        Ok(())
    }

    /// Load persisted parameters from SQLite. Returns `Self::new()` if not found.
    pub fn load(conn: &Connection) -> SqlResult<Self> {
        let result: SqlResult<String> = conn.query_row(
            "SELECT param_json FROM model_params WHERE param_key = 'adaptive_thresholds'",
            [],
            |row| row.get(0),
        );
        match result {
            Ok(json) => {
                let at: AdaptiveThresholds = serde_json::from_str(&json)
                    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    ))?;
                info!(
                    metrics = at.stats.len(),
                    calibration_pct = (at.calibration_fraction() * 100.0) as u32,
                    "AdaptiveThresholds loaded from DB"
                );
                Ok(at)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                debug!("No saved AdaptiveThresholds found — starting fresh");
                Ok(AdaptiveThresholds::new())
            }
            Err(e) => Err(e),
        }
    }
}

impl Default for AdaptiveThresholds {
    fn default() -> Self { Self::new() }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welford_mean_and_variance() {
        let mut s = WelfordState::new();
        for v in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            s.update(v);
        }
        // Classic example: mean=5.0, variance=4.0, stddev=2.0
        assert!((s.mean - 5.0).abs() < 1e-10);
        assert!((s.variance() - 4.0).abs() < 1e-10);
        assert!((s.stddev() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn threshold_requires_min_samples() {
        let mut at = AdaptiveThresholds::new();
        // Feed 49 samples — should still return None
        for _ in 0..49 {
            at.update("cpu", 50.0);
        }
        assert!(at.threshold("cpu", 2.5).is_none());
        assert!(!at.is_calibrated("cpu"));
        assert_eq!(at.sample_count("cpu"), 49);

        // 50th sample — should now return Some
        at.update("cpu", 50.0);
        assert!(at.threshold("cpu", 2.5).is_some());
        assert!(at.is_calibrated("cpu"));
    }

    #[test]
    fn threshold_value_is_mean_plus_k_sigma() {
        let mut at = AdaptiveThresholds::new();
        // Constant signal: mean=70, stddev=0 → threshold = 70 at any sigma
        for _ in 0..MIN_SAMPLES {
            at.update("cpu", 70.0);
        }
        let thr = at.threshold("cpu", 2.5).unwrap();
        assert!((thr - 70.0).abs() < 1e-9, "constant signal threshold should equal mean");
    }

    #[test]
    fn threshold_adapts_to_high_baseline_machine() {
        let mut at = AdaptiveThresholds::new();
        // Simulate a render farm that runs at ~80% CPU normally
        // Feed values around 80 with stddev ~5
        let values: Vec<f64> = (0..MIN_SAMPLES)
            .map(|i| 80.0 + ((i % 10) as f64 - 4.5))
            .collect();
        for v in &values {
            at.update("cpu_usage_avg_5m", *v);
        }
        let thr = at.threshold("cpu_usage_avg_5m", 2.5).unwrap();
        // Should be well above the hard-coded default of 85%
        // mean ≈ 80, stddev ≈ 2.9, threshold ≈ 87.2
        assert!(thr > 85.0, "high-baseline machine should raise threshold above 85%: got {}", thr);
    }

    #[test]
    fn threshold_low_baseline_machine_is_more_sensitive() {
        let mut at = AdaptiveThresholds::new();
        // Laptop that normally idles at 15% CPU
        for i in 0..MIN_SAMPLES {
            at.update("cpu_usage_avg_5m", 15.0 + (i % 5) as f64);
        }
        let thr = at.threshold("cpu_usage_avg_5m", 2.5).unwrap();
        // Should be well below 85% — this machine would flag at ~25%
        assert!(thr < 85.0, "low-baseline machine should have lower threshold: got {}", thr);
    }

    #[test]
    fn unknown_metric_returns_none() {
        let at = AdaptiveThresholds::new();
        assert!(at.threshold("nonexistent_metric", 2.5).is_none());
    }

    #[test]
    fn calibration_fraction_correct() {
        let mut at = AdaptiveThresholds::new();
        // Calibrate "cpu" only
        for _ in 0..MIN_SAMPLES {
            at.update("cpu", 50.0);
        }
        // Add uncalibrated "memory"
        at.update("memory", 60.0);

        let frac = at.calibration_fraction();
        assert!((frac - 0.5).abs() < 1e-9, "1 of 2 metrics calibrated = 0.5, got {}", frac);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Create the model_params table (normally done by migrations)
        conn.execute_batch(
            "CREATE TABLE model_params (
                param_key   TEXT PRIMARY KEY,
                param_json  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );"
        ).unwrap();

        let mut at = AdaptiveThresholds::new();
        for _ in 0..MIN_SAMPLES {
            at.update("cpu_usage_avg_5m", 55.0);
            at.update("memory_used_pct", 70.0);
        }
        at.save(&conn).unwrap();

        let loaded = AdaptiveThresholds::load(&conn).unwrap();
        assert_eq!(loaded.sample_count("cpu_usage_avg_5m"), MIN_SAMPLES);
        assert_eq!(loaded.sample_count("memory_used_pct"), MIN_SAMPLES);
        let thr = loaded.threshold("cpu_usage_avg_5m", 2.5).unwrap();
        // constant signal → threshold = mean = 55.0
        assert!((thr - 55.0).abs() < 1e-9);
    }

    #[test]
    fn load_returns_fresh_when_no_saved_data() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE model_params (
                param_key   TEXT PRIMARY KEY,
                param_json  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );"
        ).unwrap();

        let at = AdaptiveThresholds::load(&conn).unwrap();
        assert_eq!(at.calibration_fraction(), 0.0);
        assert!(at.threshold("cpu_usage_avg_5m", 2.5).is_none());
    }
}
