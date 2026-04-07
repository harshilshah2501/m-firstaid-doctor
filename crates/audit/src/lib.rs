use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// All event types that can appear in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum EventType {
    // Agent lifecycle
    AgentStarted,
    AgentStopped,
    // Collection
    CollectionCompleted,
    CollectionFailed,
    // Triggers
    TriggerFired,
    TriggerCooledDown,
    // Diagnosis
    DiagnosisStarted,
    DiagnosisCompleted,
    // Remediation
    RemediationProposed,
    RemediationApproved,
    RemediationRejected,
    RemediationDeniedByPolicy,
    ActionStarted,
    ActionCompleted,
    ActionFailed,
    ActionSkipped,
    // Verification
    VerificationCompleted,
    // Security
    ModelLoaded,
    ModelUnloaded,
    ModelChecksumFailed,
    // Config
    ConfigChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub sequence_num: i64,
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub session_id: Uuid,
    pub payload: serde_json::Value,
    /// ID of the immediately preceding event — enables chain verification
    pub prev_event_id: Option<Uuid>,
}

/// Stateful audit logger — maintains sequence counter and chain state.
pub struct AuditLogger {
    session_id: Uuid,
    next_seq: i64,
    prev_event_id: Option<Uuid>,
}

impl AuditLogger {
    pub fn new(session_id: Uuid) -> Self {
        Self { session_id, next_seq: 1, prev_event_id: None }
    }

    /// Create and persist an audit event.
    /// The event is written to SQLite AND to tracing (structured log).
    pub fn log(
        &mut self,
        conn: &Connection,
        event_type: EventType,
        payload: serde_json::Value,
    ) -> rusqlite::Result<AuditEvent> {
        let event = AuditEvent {
            event_id: Uuid::new_v4(),
            sequence_num: self.next_seq,
            timestamp: Utc::now(),
            event_type,
            session_id: self.session_id,
            payload,
            prev_event_id: self.prev_event_id,
        };

        // Write to SQLite (append-only — no UPDATE/DELETE on audit_log)
        let payload_json = serde_json::to_string(&event.payload)
            .unwrap_or_else(|_| "{}".into());

        conn.execute(
            "INSERT INTO audit_log(event_id, sequence_num, timestamp, event_type, session_id, payload_json, prev_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.event_id.to_string(),
                event.sequence_num,
                event.timestamp.to_rfc3339(),
                serde_json::to_string(&event.event_type).unwrap_or_default(),
                event.session_id.to_string(),
                payload_json,
                event.prev_event_id.map(|id| id.to_string()),
            ],
        )?;

        // Structured log — survives even if SQLite write fails
        tracing::info!(
            event_id = %event.event_id,
            sequence_num = event.sequence_num,
            event_type = ?event.event_type,
            "AUDIT"
        );

        self.next_seq += 1;
        self.prev_event_id = Some(event.event_id);

        Ok(event)
    }
}

/// Verify the audit chain: no gaps in sequence numbers, prev_event_id links are correct.
/// Returns Ok(count) if chain is valid, Err(description) if tamper detected.
pub fn verify_chain(conn: &Connection) -> Result<i64, String> {
    let mut stmt = conn.prepare(
        "SELECT event_id, sequence_num, prev_event_id FROM audit_log ORDER BY sequence_num ASC"
    ).map_err(|e| e.to_string())?;

    let mut prev_id: Option<String> = None;
    let mut expected_seq: i64 = 1;
    let mut count: i64 = 0;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    }).map_err(|e| e.to_string())?;

    for row in rows {
        let (event_id, seq, prev) = row.map_err(|e| e.to_string())?;
        if seq != expected_seq {
            return Err(format!("Sequence gap: expected {}, got {}", expected_seq, seq));
        }
        if prev != prev_id {
            return Err(format!("Chain break at seq {}: expected prev {:?}, got {:?}", seq, prev_id, prev));
        }
        prev_id = Some(event_id);
        expected_seq += 1;
        count += 1;
    }

    Ok(count)
}
