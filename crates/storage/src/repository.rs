use rusqlite::{Connection, params, Result as SqlResult};
use signal_normalizer::NormalizedSignalSnapshot;
use detector::types::DiagnosisOutput;
use remediation::{RemediationPlan, ExecutionResult};
use uuid::Uuid;

/// Repository wraps a borrowed SQLite connection and provides typed read/write operations.
pub struct Repository<'conn> {
    conn: &'conn Connection,
}

impl<'conn> Repository<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    // ── Signal Snapshots ───────────────────────────────────────────────────

    pub fn save_snapshot(&self, snapshot: &NormalizedSignalSnapshot) -> SqlResult<()> {
        let json = serde_json::to_string(snapshot)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO signal_snapshots(snapshot_id, collected_at, platform, snapshot_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                snapshot.snapshot_id.to_string(),
                snapshot.collected_at.to_rfc3339(),
                format!("{:?}", snapshot.platform),
                json,
            ],
        )?;
        Ok(())
    }

    /// Return the N most recent snapshots for rolling-window feature extraction.
    pub fn recent_snapshots(&self, limit: usize) -> SqlResult<Vec<NormalizedSignalSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT snapshot_json FROM signal_snapshots ORDER BY collected_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let json: String = row.get(0)?;
            serde_json::from_str(&json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
            })
        })?;
        rows.collect()
    }

    /// Delete snapshots older than `days` days.
    pub fn prune_old_snapshots(&self, days: u32) -> SqlResult<usize> {
        self.conn.execute(
            "DELETE FROM signal_snapshots WHERE collected_at < datetime('now', ?1)",
            params![format!("-{} days", days)],
        )
    }

    // ── Diagnoses ──────────────────────────────────────────────────────────

    pub fn save_diagnosis(&self, diag: &DiagnosisOutput) -> SqlResult<()> {
        let json = serde_json::to_string(diag)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let classes: Vec<String> = diag.diagnoses.iter()
            .map(|d| format!("{:?}", d.issue_class))
            .collect();
        let classes_json = serde_json::to_string(&classes)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let max_severity = diag.max_severity().map(|s| format!("{:?}", s));

        self.conn.execute(
            "INSERT OR REPLACE INTO diagnoses(diagnosis_id, snapshot_id, diagnosed_at, issue_classes, max_severity, output_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                diag.diagnosis_id.to_string(),
                diag.snapshot_id.to_string(),
                diag.diagnosed_at.to_rfc3339(),
                classes_json,
                max_severity,
                json,
            ],
        )?;
        Ok(())
    }

    // ── Remediation Plans ──────────────────────────────────────────────────

    pub fn save_plan(&self, plan: &RemediationPlan) -> SqlResult<()> {
        let json = serde_json::to_string(plan)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO remediation_plans(plan_id, diagnosis_id, planned_at, plan_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                plan.plan_id.to_string(),
                plan.diagnosis_id.to_string(),
                plan.planned_at.to_rfc3339(),
                json,
            ],
        )?;
        Ok(())
    }

    pub fn save_execution_result(&self, result: &ExecutionResult) -> SqlResult<()> {
        let json = serde_json::to_string(result)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO execution_results(result_id, plan_id, executed_at, overall_status, result_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                result.result_id.to_string(),
                result.plan_id.to_string(),
                result.executed_at.to_rfc3339(),
                format!("{:?}", result.overall_status),
                json,
            ],
        )?;
        Ok(())
    }

    // ── Known Issues (recurrence) ──────────────────────────────────────────

    pub fn record_issue_occurrence(&self, issue_class: &str) -> SqlResult<()> {
        self.conn.execute(
            r#"INSERT INTO known_issues(issue_class, first_seen_at, last_seen_at, occurrence_count)
               VALUES (?1, datetime('now'), datetime('now'), 1)
               ON CONFLICT(issue_class) DO UPDATE SET
                   last_seen_at = datetime('now'),
                   occurrence_count = occurrence_count + 1"#,
            params![issue_class],
        )?;
        Ok(())
    }

    pub fn get_occurrence_count(&self, issue_class: &str) -> SqlResult<u32> {
        self.conn.query_row(
            "SELECT occurrence_count FROM known_issues WHERE issue_class = ?1",
            params![issue_class],
            |row| row.get::<_, i64>(0).map(|v| v as u32),
        ).or(Ok(0))
    }
}
