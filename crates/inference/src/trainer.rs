//! On-device Isolation Forest retraining utilities.
//!
//! # Concept
//! Every collection cycle, the top-CPU process observation is recorded to
//! `process_observations` in SQLite. Once a week (configurable), this module
//! spawns `scripts/train_process_anomaly.py` as a subprocess which reads the
//! last 7 days of observations, trains a GBC-calibrated Isolation Forest,
//! and writes a new `process_anomaly_v1.onnx` file. On success the manifest
//! SHA-256 is updated and the session pool naturally evicts the stale session
//! on the next eviction cycle.
//!
//! # Feature vector (5 floats)
//! ```text
//! [cpu_pct, mem_mb, io_bytes_per_sec, process_age_hours, cpu_trend_5m]
//! ```
//! `io_bytes_per_sec` and `process_age_hours` are 0.0 until a future collector
//! exposes per-process I/O and start-time data.
//!
//! # Thread safety
//! All functions accept `&Connection` (non-Send, single-threaded).
//! Call from the orchestrator's non-async context.

use rusqlite::{params, Connection, Result as SqlResult};
use tracing::{debug, info, warn};

// ── Feature vector ────────────────────────────────────────────────────────────

/// Feature names in order (for documentation and training script headers).
pub const FEATURE_NAMES: [&str; 5] = [
    "cpu_pct",
    "mem_mb",
    "io_bytes_per_sec",
    "process_age_hours",
    "cpu_trend_5m",
];

/// Minimum observations required before attempting retraining.
/// Below this count the synthetic pre-trained model is kept as-is.
pub const MIN_OBS_FOR_RETRAIN: usize = 100;

/// Default retraining interval in days.
pub const RETRAIN_INTERVAL_DAYS: u32 = 7;

// ── Observation recording ─────────────────────────────────────────────────────

/// Record one process observation to the SQLite rolling log.
///
/// `features` must follow the canonical layout:
/// `[cpu_pct, mem_mb, io_bytes_per_sec, process_age_hours, cpu_trend_5m]`
///
/// Automatically prunes records older than 30 days (non-fatal on prune error).
pub fn record_process_observation(
    conn: &Connection,
    process_name: &str,
    features: &[f32; 5],
) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO process_observations
         (process_name, cpu_pct, mem_mb, io_bytes_per_sec, process_age_hours, cpu_trend_5m)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            process_name,
            features[0] as f64,
            features[1] as f64,
            features[2] as f64,
            features[3] as f64,
            features[4] as f64,
        ],
    )?;

    // Non-fatal prune — stale rows just waste space, not correctness.
    if let Err(e) = conn.execute(
        "DELETE FROM process_observations WHERE collected_at < datetime('now', '-30 days')",
        [],
    ) {
        warn!(error = %e, "Could not prune old process observations");
    }

    Ok(())
}

/// Count observations in the rolling log for a given process (or all if `None`).
pub fn observation_count(conn: &Connection, process_name: Option<&str>) -> SqlResult<usize> {
    let count: i64 = match process_name {
        Some(name) => conn.query_row(
            "SELECT COUNT(*) FROM process_observations WHERE process_name = ?1",
            params![name],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM process_observations",
            [],
            |row| row.get(0),
        )?,
    };
    Ok(count as usize)
}

// ── Retrain scheduling ────────────────────────────────────────────────────────

/// Returns `true` if retraining is overdue for `model_name`.
///
/// Overdue means either:
/// - No retrain record exists (first run), **and** enough observations exist, or
/// - The last retrain was more than `RETRAIN_INTERVAL_DAYS` ago.
pub fn should_retrain(conn: &Connection, model_name: &str) -> bool {
    // Check if we have enough observations first (avoids spurious retrains on fresh installs).
    let obs = observation_count(conn, None).unwrap_or(0);
    if obs < MIN_OBS_FOR_RETRAIN {
        debug!(
            obs,
            min = MIN_OBS_FOR_RETRAIN,
            "Too few observations for retraining — keeping pre-trained model"
        );
        return false;
    }

    let due: bool = conn.query_row(
        "SELECT CASE
             WHEN last_trained_at IS NULL THEN 1
             WHEN julianday('now') - julianday(last_trained_at) >= ?2 THEN 1
             ELSE 0
         END
         FROM model_retrain_log WHERE model_name = ?1",
        params![model_name, RETRAIN_INTERVAL_DAYS as f64],
        |row| row.get(0),
    ).unwrap_or(true); // no record → treat as overdue

    due
}

/// Record a completed retrain in `model_retrain_log`.
pub fn record_retrain(
    conn: &Connection,
    model_name: &str,
    obs_count: usize,
    onnx_sha256: Option<&str>,
) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO model_retrain_log(model_name, last_trained_at, obs_count, onnx_sha256)
         VALUES (?1, datetime('now'), ?2, ?3)
         ON CONFLICT(model_name) DO UPDATE SET
             last_trained_at = excluded.last_trained_at,
             obs_count       = excluded.obs_count,
             onnx_sha256     = excluded.onnx_sha256",
        params![model_name, obs_count as i64, onnx_sha256],
    )?;
    Ok(())
}

// ── Subprocess retraining ─────────────────────────────────────────────────────

/// Spawn `scripts/train_process_anomaly.py` as a background subprocess.
///
/// The script reads `process_observations` from `db_path`, trains the model,
/// writes the ONNX file, and updates `models/manifest.json`. The Orchestrator's
/// next eviction cycle will drop the stale session so the new model is loaded
/// on the next inference call.
///
/// Returns the number of observations passed to training, or an error.
/// The subprocess is fire-and-forget — we do not wait for it.
pub fn spawn_retrain_subprocess(
    db_path: &std::path::Path,
    script_path: &std::path::Path,
    model_output: &std::path::Path,
    manifest_path: &std::path::Path,
) -> Result<(), std::io::Error> {
    let obs = std::process::Command::new("python3")
        .arg(script_path)
        .arg("--db")
        .arg(db_path)
        .arg("--output")
        .arg(model_output)
        .arg("--manifest")
        .arg(manifest_path)
        .spawn()?;

    info!(
        pid = obs.id(),
        script = %script_path.display(),
        "Isolation Forest retraining subprocess spawned"
    );
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Create the tables needed by the trainer module.
        conn.execute_batch(
            "CREATE TABLE process_observations (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                collected_at     TEXT NOT NULL DEFAULT (datetime('now')),
                process_name     TEXT NOT NULL,
                cpu_pct          REAL NOT NULL,
                mem_mb           REAL NOT NULL,
                io_bytes_per_sec REAL NOT NULL DEFAULT 0.0,
                process_age_hours REAL NOT NULL DEFAULT 0.0,
                cpu_trend_5m     REAL NOT NULL DEFAULT 0.0
             );
             CREATE INDEX idx_proc ON process_observations(process_name, collected_at DESC);
             CREATE TABLE model_retrain_log (
                model_name      TEXT PRIMARY KEY,
                last_trained_at TEXT NOT NULL DEFAULT (datetime('now')),
                obs_count       INTEGER NOT NULL DEFAULT 0,
                onnx_sha256     TEXT
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn record_and_count_observations() {
        let conn = in_memory_db();
        let features = [75.0f32, 512.0, 0.0, 0.0, 5.0];
        record_process_observation(&conn, "myapp", &features).unwrap();
        record_process_observation(&conn, "myapp", &features).unwrap();
        record_process_observation(&conn, "otherapp", &features).unwrap();

        assert_eq!(observation_count(&conn, Some("myapp")).unwrap(), 2);
        assert_eq!(observation_count(&conn, Some("otherapp")).unwrap(), 1);
        assert_eq!(observation_count(&conn, None).unwrap(), 3);
    }

    #[test]
    fn should_retrain_false_when_too_few_obs() {
        let conn = in_memory_db();
        // 0 observations → should not retrain
        assert!(!should_retrain(&conn, "process_anomaly_v1"));
    }

    #[test]
    fn should_retrain_true_when_no_log_and_enough_obs() {
        let conn = in_memory_db();
        let features = [10.0f32, 100.0, 0.0, 0.0, 0.0];
        for _ in 0..MIN_OBS_FOR_RETRAIN {
            record_process_observation(&conn, "app", &features).unwrap();
        }
        // No retrain log entry → should retrain
        assert!(should_retrain(&conn, "process_anomaly_v1"));
    }

    #[test]
    fn should_retrain_false_after_recent_retrain() {
        let conn = in_memory_db();
        let features = [10.0f32, 100.0, 0.0, 0.0, 0.0];
        for _ in 0..MIN_OBS_FOR_RETRAIN {
            record_process_observation(&conn, "app", &features).unwrap();
        }
        record_retrain(&conn, "process_anomaly_v1", MIN_OBS_FOR_RETRAIN, None).unwrap();
        // Just retrained → not due again yet
        assert!(!should_retrain(&conn, "process_anomaly_v1"));
    }

    #[test]
    fn record_retrain_upserts_correctly() {
        let conn = in_memory_db();
        record_retrain(&conn, "process_anomaly_v1", 120, Some("abc123")).unwrap();
        record_retrain(&conn, "process_anomaly_v1", 200, Some("def456")).unwrap();

        let sha: String = conn.query_row(
            "SELECT onnx_sha256 FROM model_retrain_log WHERE model_name = 'process_anomaly_v1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(sha, "def456", "second upsert should overwrite first");
    }
}
