use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────────────────────────────────────
// Remediation Plan
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationPlan {
    pub schema_version: String,
    pub plan_id: Uuid,
    pub diagnosis_id: Uuid,
    pub planned_at: DateTime<Utc>,
    /// Actions that can be executed (gated by policy)
    pub actions: Vec<RemediationAction>,
    /// Recommendations that will never be automatically executed
    pub recommended_only: Vec<Recommendation>,
}

impl RemediationPlan {
    pub fn new(diagnosis_id: Uuid) -> Self {
        Self {
            schema_version: "1.0".into(),
            plan_id: Uuid::new_v4(),
            diagnosis_id,
            planned_at: Utc::now(),
            actions: Vec::new(),
            recommended_only: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty() && self.recommended_only.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationAction {
    pub action_id: Uuid,
    pub issue_class: String,
    pub action_type: ActionType,
    pub title: String,
    pub description: String,
    pub safety_level: SafetyLevel,
    pub privilege_required: PrivilegeLevel,
    pub estimated_risk: RiskLevel,
    pub reversible: bool,
    pub rollback_note: String,
    pub pre_checks: Vec<String>,
}

impl RemediationAction {
    pub fn new(action_type: ActionType, title: impl Into<String>) -> Self {
        Self {
            action_id: Uuid::new_v4(),
            issue_class: String::new(),
            action_type,
            title: title.into(),
            description: String::new(),
            safety_level: SafetyLevel::ApprovalRequired,
            privilege_required: PrivilegeLevel::User,
            estimated_risk: RiskLevel::Low,
            reversible: false,
            rollback_note: String::new(),
            pre_checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub issue_class: String,
    pub title: String,
    pub description: String,
    pub safety_level: SafetyLevel,
}

// ──────────────────────────────────────────────────────────────────────────────
// Action types — typed, never a shell string
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    /// Report diagnostic information to the user (always AutoSafe)
    Report { message: String },
    /// Flush DNS resolver cache
    FlushDnsCache,
    /// Kill a process by PID with optional fallback signal
    KillProcess { pid: u32, process_name: String, signal: Signal, fallback: Option<Signal> },
    /// Renice (adjust CPU priority) of a process
    ReprioritizeProcess { pid: u32, nice_value: i8 },
    /// Clear files from approved cache paths
    ClearCache { target_paths: Vec<String>, min_age_days: u32, dry_run: bool },
    /// Disable a startup/login item
    DisableStartupItem { name: String, source: String },
    /// Enable a startup/login item (undo of disable)
    EnableStartupItem { name: String, source: String },
    /// Restart a named service
    RestartService { service_name: String },
    /// Stop a service
    StopService { service_name: String },
    /// Run a connectivity diagnostic probe (read-only)
    RunConnectivityProbe { probe_type: ProbeType },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Signal {
    Sigterm,
    Sigkill,
    Sigstop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProbeType {
    DnsLookup { domain: String },
    PingGateway,
    PingPublicDns,
}

// ──────────────────────────────────────────────────────────────────────────────
// Safety / Privilege / Risk enums (mirrors policy crate)
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SafetyLevel {
    AutoSafe         = 0,
    ApprovalRequired = 1,
    ElevationRequired = 2,
    NeverAutoFix     = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrivilegeLevel {
    User,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskLevel {
    Negligible,
    Low,
    Medium,
    High,
}

// ──────────────────────────────────────────────────────────────────────────────
// Execution Result
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub schema_version: String,
    pub result_id: Uuid,
    pub plan_id: Uuid,
    pub executed_at: DateTime<Utc>,
    pub consent_obtained: bool,
    pub consent_method: ConsentMethod,
    pub actions: Vec<ActionResult>,
    pub overall_status: OverallStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub action_id: Uuid,
    pub status: ActionStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub pre_check_results: Vec<PreCheckResult>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreCheckResult {
    pub check: String,
    pub passed: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionStatus {
    Success,
    Failed,
    SkippedPreCheck,
    Denied,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConsentMethod {
    AutoApproved,    // AutoSafe action
    UserApproval,    // User explicitly approved
    PolicyOverride,  // MDM policy pre-approved (Phase 4)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OverallStatus {
    Success,
    PartialSuccess,
    Failed,
    Denied,
}
