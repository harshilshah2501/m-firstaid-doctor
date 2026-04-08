//! Core diagnostic pipeline orchestrator.
//! Coordinates: collect → normalize → feature-extract → trigger → diagnose → plan → gate → execute → verify → audit

use crate::config::AgentConfig;
use audit::{AuditLogger, EventType};
use collectors::{CollectorSet, Platform};
use detector::{diagnose, merge_ml};
use feature_extractor::{extract, to_feature_vec};
use inference::{
    AdaptiveThresholds, ModelSessionPool,
    record_process_observation, should_retrain,
    temporal,
};
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
    /// Rolling window of feature vectors (oldest first) for temporal anomaly scoring.
    /// Capped at `temporal::T_MAX` entries; one entry per collection cycle.
    feature_history: Vec<Vec<f32>>,
}

impl Orchestrator {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            collector_set: CollectorSet::new(Platform::current()),
            trigger_engine: TriggerEngine::new(),
            session_id: Uuid::new_v4(),
            ml_pool: ModelSessionPool::new(),
            feature_history: Vec::with_capacity(temporal::T_MAX),
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

        // Append current feature vector to the in-memory temporal history ring.
        // Oldest entries are evicted when the buffer exceeds T_MAX.
        let fvec = to_feature_vec(&features);
        if self.feature_history.len() >= temporal::T_MAX {
            self.feature_history.remove(0);
        }
        self.feature_history.push(fvec);

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

        // ── Step 6b: ML augmentation (Phase D/E) ─────────────────────────────
        // Build per-class ML score map. Each model result is keyed by IssueClass.
        // Both functions return Ok(None) gracefully when onnx is off or model absent.
        let mut ml_scores = HashMap::new();

        // Phase D — CharCNN crash classifier
        if let Some(log_text) = &features.crash_log_text {
            match inference::classify_crash_log(log_text, None, None) {
                Ok(Some(ir)) => { ml_scores.insert(IssueClass::AppCrash, ir); }
                Ok(None)     => {}
                Err(e)       => { warn!(error = %e, "Crash classifier failed — rules only"); }
            }
        }

        // Phase E — Isolation Forest process anomaly scorer
        // Feature vector: [cpu_pct, mem_mb, io_bytes_per_sec, process_age_hours, cpu_trend_5m]
        // io_bytes_per_sec and process_age_hours are 0.0 until future collectors expose them.
        let proc_features: [f32; 5] = [
            features.top_process_cpu_pct,
            features.top_process_rss_mb as f32,
            0.0,  // io_bytes_per_sec — not yet collected per-process
            0.0,  // process_age_hours — not yet collected
            features.cpu_usage_avg_5m - features.cpu_usage_avg_1m,  // cpu_trend_5m
        ];

        // Record every cycle for the weekly retrainer (non-fatal if DB write fails).
        if !features.top_process_name.is_empty() {
            if let Err(e) = record_process_observation(conn, &features.top_process_name, &proc_features) {
                debug!(error = %e, "Could not record process observation");
            }
        }

        // Run anomaly scorer (Ok(None) when model not configured).
        match inference::detect_process_anomaly(&proc_features, None, None) {
            Ok(Some(ir)) => { ml_scores.insert(IssueClass::HighCpu, ir); }
            Ok(None)     => {}
            Err(e)       => { warn!(error = %e, "Process anomaly scorer failed — rules only"); }
        }

        // Phase E — weekly on-device retraining check
        if should_retrain(conn, "process_anomaly_v1") {
            info!("Isolation Forest retraining is due — spawning subprocess");
            // Script and path resolution happens in a follow-up config wiring task.
            // For now, log the intent without blocking the diagnostic cycle.
            debug!("Retrain subprocess spawn deferred until model paths are configured");
        }

        // Phase C — MiniPatch-TST temporal anomaly scorer
        // Score the full feature history window (Ok(None) until model is loaded
        // or while history is too short — both are handled gracefully).
        let temporal_score = match inference::score_telemetry_window(
            &self.feature_history,
            None,   // model config wired when manifest loader is integrated
            None,
        ) {
            Ok(ts) => ts,
            Err(e) => {
                warn!(error = %e, "Temporal scorer failed — rules only");
                None
            }
        };

        if let Some(ref ts) = temporal_score {
            debug!(
                anomaly_score = ts.anomaly_score,
                history_len   = self.feature_history.len(),
                "Temporal anomaly score computed"
            );
        }

        let diagnosis = merge_ml(rule_output, &ml_scores, temporal_score.as_ref());

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
