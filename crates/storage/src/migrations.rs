use rusqlite::{Connection, Result as SqlResult};
use tracing::debug;

/// Apply all schema migrations in order.
/// Each migration is idempotent (CREATE TABLE IF NOT EXISTS).
pub fn apply_all(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(MIGRATION_001)?;
    debug!("Migrations applied");
    Ok(())
}

/// Migration 001 — initial schema
const MIGRATION_001: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL
);

-- Rolling 7-day snapshot history (normalized snapshots stored as compressed JSON)
CREATE TABLE IF NOT EXISTS signal_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    collected_at TEXT NOT NULL,
    platform    TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,   -- NormalizedSignalSnapshot as JSON
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_snapshots_collected ON signal_snapshots(collected_at DESC);

-- Diagnosis history
CREATE TABLE IF NOT EXISTS diagnoses (
    diagnosis_id TEXT PRIMARY KEY,
    snapshot_id  TEXT NOT NULL,
    diagnosed_at TEXT NOT NULL,
    issue_classes TEXT NOT NULL,   -- JSON array of issue class names
    max_severity TEXT,
    output_json  TEXT NOT NULL,    -- DiagnosisOutput as JSON
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (snapshot_id) REFERENCES signal_snapshots(snapshot_id)
);
CREATE INDEX IF NOT EXISTS idx_diagnoses_at ON diagnoses(diagnosed_at DESC);

-- Remediation plans
CREATE TABLE IF NOT EXISTS remediation_plans (
    plan_id      TEXT PRIMARY KEY,
    diagnosis_id TEXT NOT NULL,
    planned_at   TEXT NOT NULL,
    plan_json    TEXT NOT NULL,   -- RemediationPlan as JSON
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (diagnosis_id) REFERENCES diagnoses(diagnosis_id)
);

-- Execution results
CREATE TABLE IF NOT EXISTS execution_results (
    result_id   TEXT PRIMARY KEY,
    plan_id     TEXT NOT NULL,
    executed_at TEXT NOT NULL,
    overall_status TEXT NOT NULL,
    result_json TEXT NOT NULL,   -- ExecutionResult as JSON
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (plan_id) REFERENCES remediation_plans(plan_id)
);

-- Audit log — APPEND ONLY — no UPDATE or DELETE ever issued against this table
CREATE TABLE IF NOT EXISTS audit_log (
    event_id     TEXT PRIMARY KEY,
    sequence_num INTEGER NOT NULL,
    timestamp    TEXT NOT NULL,
    event_type   TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    prev_event_id TEXT,   -- chain link
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_audit_seq ON audit_log(sequence_num ASC);
CREATE INDEX IF NOT EXISTS idx_audit_ts  ON audit_log(timestamp DESC);

-- Issue recurrence tracking
CREATE TABLE IF NOT EXISTS known_issues (
    issue_class         TEXT PRIMARY KEY,
    first_seen_at       TEXT NOT NULL,
    last_seen_at        TEXT NOT NULL,
    occurrence_count    INTEGER NOT NULL DEFAULT 1,
    last_remediation_at TEXT,
    remediation_count   INTEGER NOT NULL DEFAULT 0,
    escalated           INTEGER NOT NULL DEFAULT 0  -- boolean
);

INSERT OR IGNORE INTO schema_version(version, applied_at)
VALUES (1, datetime('now'));
"#;
