//! Core diagnostic pipeline orchestrator.
//! Coordinates: collect → normalize → feature-extract → trigger → diagnose → plan → gate → execute → verify → audit

use crate::config::AgentConfig;
use audit::{AuditLogger, EventType};
use collectors::{CollectorSet, Platform};
use detector::{diagnose, merge_ml};
use feature_extractor::extract;
use inference::{AdaptiveThresholds, ModelSessionPool};
use remediation::{execute as execute_plan, plan as build_plan, ConsentMethod};
use signal_normalizer::normalize;
use storage::Repository;
use trigger_engine::{IssueClass, TriggerEngine};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct Orchestrator {
    config: AgentConfig,
    collector_set: CollectorSet,
    trigger_engine: TriggerEngine,
    session_id: Uuid,
    /// Lazy-loading ONNX session pool — zero cost when `onnx` feature is off.
    ml_pool: ModelSessionPool,
}

impl Orchestrator {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            collector_set: CollectorSet::new(Platform::current()),
            trigger_engine: TriggerEngine::new(),
            session_id: Uuid::new_v4(),
            ml_pool: ModelSessionPool::new(),
            config,
        }
    }

    /// Run a single on-demand diagnostic cycle.
    /// Used by `firstaid diagnose` CLI command.
    pub async fn run_once(&mut self, conn: &Connection) -> Result<DiagnosticCycleResult> {
        let mut audit = AuditLogger::new(self.session_id);
        self.run_cycle_internal(conn, &mut audit).await
    }

    /// Run the agent in daemon mode — poll every N seconds.
    pub async fn run_daemon(&mut self, conn: &Connection) -> Result<()> {
        let mut audit = AuditLogger::new(self.session_id);
        let interval = Duration::from_secs(self.config.poll_interval_seconds);

        audit.log(conn, EventType::AgentStarted, serde_json::json!({
            "session_id": self.session_id,
            "poll_interval_seconds": self.config.poll_interval_seconds,
        }))?;

        info!(session_id = %self.session_id, "Agent daemon started");

        loop {
            match self.run_cycle_internal(conn, &mut audit).await {
                Ok(result) => {
                    if !result.diagnoses_found.is_empty() {
                        info!(
                            issues = ?result.diagnoses_found,
                            "Diagnostic cycle found issues"
                        );
                    } else {
                        debug!("Diagnostic cycle: no issues");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Diagnostic cycle error");
                }
            }

            sleep(interval).await;
        }
    }

    async fn run_cycle_internal(
        &mut self,
        conn: &Connection,
        audit: &mut AuditLogger,
    ) -> Result<DiagnosticCycleResult> {
        // ── Step 1: Collect ──────────────────────────────────────────────────
        debug!("Collecting signals");
        let raw = self.collector_set.collect_all().await;

        // ── Step 2: Normalize ────────────────────────────────────────────────
        let normalized = normalize(&raw);

        // ── Step 3: Persist snapshot ─────────────────────────────────────────
        let repo = Repository::new_ref(conn);
        repo.save_snapshot(&normalized)?;

        audit.log(conn, EventType::CollectionCompleted, serde_json::json!({
            "snapshot_id": normalized.snapshot_id,
            "collector_errors": raw.collector_errors.len(),
        }))?;

        // ── Step 4: Feature extraction (with history window) ─────────────────
        let history = repo.recent_snapshots(feature_extractor::HISTORY_WINDOW)?;
        let features = extract(&normalized, &history[1..]); // exclude current from history

        // ── Step 5: Adaptive threshold update + trigger evaluation ───────────
        // Load persisted thresholds, update with current observation, persist.
        // The trigger engine uses the updated thresholds for this cycle.
        // On error (e.g. table missing), fall back to rule defaults silently.
        let adaptive = AdaptiveThresholds::load(conn).ok().map(|mut at| {
            at.update_all(&[
                ("cpu_usage_avg_5m",  features.cpu_usage_avg_5m as f64),
                ("cpu_usage_avg_15m", features.cpu_usage_avg_15m as f64),
                ("memory_used_pct",   features.memory_used_pct as f64),
                ("swap_used_pct",     features.swap_used_pct as f64),
            ]);
            let _ = at.save(conn);  // non-fatal if DB write fails
            at
        });

        debug!(
            calibration_pct = adaptive.as_ref().map(|a| (a.calibration_fraction() * 100.0) as u32),
            "AdaptiveThresholds calibration status"
        );

        let trigger = self.trigger_engine.evaluate(&features, adaptive.as_ref());
        let Some(trigger_event) = trigger else {
            return Ok(DiagnosticCycleResult {
                snapshot_id: normalized.snapshot_id,
                triggered: false,
                diagnoses_found: Vec::new(),
                plan_id: None,
                executed: false,
            });
        };

        audit.log(conn, EventType::TriggerFired, serde_json::json!({
            "triggered_classes": trigger_event.issue_classes,
            "reasons": trigger_event.reasons,
        }))?;

        // ── Step 6: Diagnose ─────────────────────────────────────────────────
        audit.log(conn, EventType::DiagnosisStarted, serde_json::json!({
            "snapshot_id": normalized.snapshot_id,
        }))?;

        let rule_output = diagnose(&features, &trigger_event.issue_classes);

        // ── Step 6b: ML augmentation (Phase D — crash classifier) ────────────
        // Build per-class ML score map. Each model result is keyed by IssueClass.
        // Returns Ok(None) when the onnx feature is off or model is not loaded.
        let mut ml_scores = HashMap::new();
        if let Some(log_text) = &features.crash_log_text {
            // Crash classifier config is loaded from the manifest at startup in Phase B+.
            // Until the manifest loader is wired here, pass None — classify_crash_log
            // returns Ok(None) gracefully and rules-only mode applies.
            match inference::classify_crash_log(log_text, None, None) {
                Ok(Some(ir)) => { ml_scores.insert(IssueClass::AppCrash, ir); }
                Ok(None)     => {}
                Err(e)       => { warn!(error = %e, "Crash classifier failed — rules only"); }
            }
        }

        let diagnosis = merge_ml(rule_output, &ml_scores, None);

        if diagnosis.is_empty() {
            debug!("Trigger fired but no diagnosis produced — inconclusive");
            return Ok(DiagnosticCycleResult {
                snapshot_id: normalized.snapshot_id,
                triggered: true,
                diagnoses_found: Vec::new(),
                plan_id: None,
                executed: false,
            });
        }

        let found_classes: Vec<String> = diagnosis.diagnoses.iter()
            .map(|d| format!("{:?}", d.issue_class))
            .collect();

        repo.save_diagnosis(&diagnosis)?;
        for class in &found_classes {
            repo.record_issue_occurrence(class)?;
        }

        audit.log(conn, EventType::DiagnosisCompleted, serde_json::json!({
            "diagnosis_id": diagnosis.diagnosis_id,
            "issues": found_classes,
            "max_severity": diagnosis.max_severity().map(|s| format!("{:?}", s)),
            "elapsed_ms": diagnosis.elapsed_ms,
        }))?;

        // ── Step 7: Remediation planning ─────────────────────────────────────
        let plan = build_plan(&diagnosis);

        if plan.is_empty() {
            return Ok(DiagnosticCycleResult {
                snapshot_id: normalized.snapshot_id,
                triggered: true,
                diagnoses_found: found_classes,
                plan_id: Some(plan.plan_id),
                executed: false,
            });
        }

        repo.save_plan(&plan)?;

        audit.log(conn, EventType::RemediationProposed, serde_json::json!({
            "plan_id": plan.plan_id,
            "action_count": plan.actions.len(),
            "recommendation_count": plan.recommended_only.len(),
        }))?;

        // ── Step 8: Execute AutoSafe actions immediately ──────────────────────
        // AutoSafe actions (safety_level=0) execute without consent.
        // ApprovalRequired actions are gated — in daemon mode, notify user.
        // In Phase 1 (CLI), we don't execute ApprovalRequired actions automatically.
        let autosafe_plan = filter_autosafe(&plan);
        let plan_id = plan.plan_id;

        let executed = if !autosafe_plan.actions.is_empty() {
            let result = execute_plan(&autosafe_plan, ConsentMethod::AutoApproved).await;
            repo.save_execution_result(&result)?;
            audit.log(conn, EventType::ActionCompleted, serde_json::json!({
                "result_id": result.result_id,
                "overall_status": format!("{:?}", result.overall_status),
            }))?;
            true
        } else {
            false
        };

        Ok(DiagnosticCycleResult {
            snapshot_id: normalized.snapshot_id,
            triggered: true,
            diagnoses_found: found_classes,
            plan_id: Some(plan_id),
            executed,
        })
    }
}

#[derive(Debug)]
pub struct DiagnosticCycleResult {
    pub snapshot_id: uuid::Uuid,
    pub triggered: bool,
    pub diagnoses_found: Vec<String>,
    pub plan_id: Option<uuid::Uuid>,
    pub executed: bool,
}

fn filter_autosafe(plan: &remediation::RemediationPlan) -> remediation::RemediationPlan {
    use remediation::SafetyLevel;
    let mut filtered = plan.clone();
    filtered.actions.retain(|a| a.safety_level == SafetyLevel::AutoSafe);
    filtered
}

// Extension trait to allow creating a non-owning ref to the repo
trait RepoExt {
    fn new_ref(conn: &Connection) -> Repository;
}
impl RepoExt for Repository {
    fn new_ref(conn: &Connection) -> Repository {
        // Phase 1: pass conn by reference via a shared connection pool.
        // For now, open a separate in-memory connection to satisfy the type system.
        // Phase 2: use r2d2 or deadpool-sqlite for proper connection management.
        todo!("Phase 1: pass conn reference — will be addressed with connection pool in Phase 2")
    }
}
