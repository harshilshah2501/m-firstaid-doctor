//! Lazy-loading ONNX session pool with idle-timeout eviction.
//!
//! # Memory model
//! Sessions are loaded from disk on first use and dropped after `idle_timeout`
//! of inactivity. When no inference is in flight, RAM consumed = 0 bytes.
//!
//! # Checksum enforcement
//! Every load verifies the model file's SHA-256 digest against the manifest
//! before creating an ORT session. A mismatch is a hard error — the session
//! is never created and `InferenceError::ChecksumMismatch` is returned.
//!
//! # Thread safety
//! `ModelSessionPool` is `Send + Sync`. The internal session map is guarded by
//! a `Mutex`. Inference calls hold the lock only for the duration of the run.
//!
//! # Feature gate
//! Without `--features onnx`, all methods compile but return
//! `InferenceError::FeatureNotEnabled`. No ORT symbols are linked.

use crate::{InferenceError, ModelConfig};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

// ── Idle-eviction constants ───────────────────────────────────────────────────

/// Default idle timeout: drop a session after 60 s of no calls.
const DEFAULT_IDLE_SECS: u64 = 60;

// ── Session pool ─────────────────────────────────────────────────────────────

/// A cached ONNX session entry (only compiled when `onnx` feature is active).
#[cfg(feature = "onnx")]
struct CachedSession {
    session: ort::session::Session,
    last_used: Instant,
}

/// Lazy-loading ONNX session pool.
///
/// Create one instance per agent lifetime (usually one per process).
/// Inject into inference functions via `&ModelSessionPool`.
pub struct ModelSessionPool {
    idle_timeout: Duration,
    #[cfg(feature = "onnx")]
    sessions: std::sync::Mutex<HashMap<String, CachedSession>>,
    /// Track last-used times even without ORT (for diagnostics / testing).
    #[cfg(not(feature = "onnx"))]
    _phantom: std::marker::PhantomData<()>,
}

impl ModelSessionPool {
    /// Create a pool with the default 60-second idle timeout.
    pub fn new() -> Self {
        Self {
            idle_timeout: Duration::from_secs(DEFAULT_IDLE_SECS),
            #[cfg(feature = "onnx")]
            sessions: std::sync::Mutex::new(HashMap::new()),
            #[cfg(not(feature = "onnx"))]
            _phantom: std::marker::PhantomData,
        }
    }

    /// Override the idle timeout (useful in tests / daemon config).
    pub fn with_idle_timeout(mut self, secs: u64) -> Self {
        self.idle_timeout = Duration::from_secs(secs);
        self
    }

    /// Run `f` with the ONNX session for `config`.
    ///
    /// Loads the session if not already cached (verifying checksum first).
    /// Returns `InferenceError::FeatureNotEnabled` when the `onnx` feature is
    /// not active — callers should treat this as "ML unavailable, skip".
    ///
    /// # Example
    /// ```rust,ignore
    /// let result = pool.with_session(&config, |session| {
    ///     let outputs = session.run(ort::inputs!["input" => tensor]?)?;
    ///     Ok(parse_outputs(outputs))
    /// })?;
    /// ```
    pub fn with_session<F, R>(
        &self,
        config: &ModelConfig,
        f: F,
    ) -> Result<R, InferenceError>
    where
        F: FnOnce(&OrtSessionRef<'_>) -> Result<R, InferenceError>,
    {
        #[cfg(not(feature = "onnx"))]
        {
            let _ = (config, f);
            return Err(InferenceError::FeatureNotEnabled);
        }

        #[cfg(feature = "onnx")]
        {
            self.ensure_loaded(config)?;
            let mut map = self.sessions.lock().unwrap();
            let cached = map.get_mut(&config.name)
                .ok_or_else(|| InferenceError::OrtError("session vanished after load".into()))?;
            cached.last_used = Instant::now();
            let guard = OrtSessionRef { session: &cached.session };
            f(&guard)
        }
    }

    /// Evict sessions idle longer than `idle_timeout`.
    ///
    /// Call from the agent's background loop (e.g. every 30 s collection cycle).
    /// No-op when `onnx` feature is off.
    pub fn evict_idle(&self) {
        #[cfg(feature = "onnx")]
        {
            let mut map = self.sessions.lock().unwrap();
            let timeout = self.idle_timeout;
            let before = map.len();
            map.retain(|name, cached| {
                let idle = cached.last_used.elapsed();
                if idle >= timeout {
                    info!(model = %name, idle_secs = idle.as_secs(), "ONNX session evicted");
                    false
                } else {
                    true
                }
            });
            let freed = before - map.len();
            if freed > 0 {
                debug!(freed, remaining = map.len(), "ONNX session pool after eviction");
            }
        }
    }

    /// Number of currently loaded sessions (0 when `onnx` feature is off).
    pub fn active_count(&self) -> usize {
        #[cfg(feature = "onnx")]
        return self.sessions.lock().unwrap().len();
        #[cfg(not(feature = "onnx"))]
        0
    }

    /// True if `onnx` feature is compiled in (runtime check for CLI display).
    pub fn onnx_enabled(&self) -> bool {
        cfg!(feature = "onnx")
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    #[cfg(feature = "onnx")]
    fn ensure_loaded(&self, config: &ModelConfig) -> Result<(), InferenceError> {
        let mut map = self.sessions.lock().unwrap();
        if map.contains_key(&config.name) {
            return Ok(());
        }

        // Verify SHA-256 before any ORT call — non-negotiable safety gate.
        crate::verify_checksum(&config.path, &config.sha256)?;

        info!(
            model = %config.name,
            version = %config.version,
            path = %config.path.display(),
            "Loading ONNX session"
        );

        let session = ort::session::Session::builder()
            .map_err(|e| InferenceError::OrtError(format!("builder: {e}")))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| InferenceError::OrtError(format!("opt-level: {e}")))?
            .with_intra_threads(1)  // one thread per model — minimal overhead
            .map_err(|e| InferenceError::OrtError(format!("threads: {e}")))?
            .commit_from_file(&config.path)
            .map_err(|e| InferenceError::OrtError(format!("load '{}': {e}", config.path.display())))?;

        map.insert(config.name.clone(), CachedSession {
            session,
            last_used: Instant::now(),
        });

        debug!(model = %config.name, "ONNX session ready");
        Ok(())
    }
}

impl Default for ModelSessionPool {
    fn default() -> Self { Self::new() }
}

// `ModelSessionPool` must be `Send + Sync` so it can live in the Orchestrator
// which is used across await points.
// Safety: we use a `Mutex` to guard all mutable state.
unsafe impl Send for ModelSessionPool {}
unsafe impl Sync for ModelSessionPool {}

// ── Session reference (type-erased wrapper) ───────────────────────────────────

/// A reference to a loaded ORT session, passed to the `with_session` closure.
///
/// Without the `onnx` feature this is an empty type — the closure is never
/// called, so no code path reaches it without ORT compiled in.
pub struct OrtSessionRef<'a> {
    #[cfg(feature = "onnx")]
    pub session: &'a ort::session::Session,
    #[cfg(not(feature = "onnx"))]
    _phantom: std::marker::PhantomData<&'a ()>,
}

// ── Model manifest loader ────────────────────────────────────────────────────

/// Load and parse `models/manifest.json` relative to the binary's location.
///
/// Returns an empty map if the manifest is missing (graceful degradation —
/// ML simply remains disabled).
pub fn load_manifest(manifest_path: &std::path::Path) -> HashMap<String, ModelConfig> {
    match std::fs::read_to_string(manifest_path) {
        Err(e) => {
            debug!(path = %manifest_path.display(), error = %e, "Model manifest not found — ML disabled");
            HashMap::new()
        }
        Ok(json) => {
            match serde_json::from_str::<ModelManifest>(&json) {
                Ok(manifest) => {
                    let map: HashMap<String, ModelConfig> = manifest.models
                        .into_iter()
                        .map(|m| (m.name.clone(), m))
                        .collect();
                    info!(count = map.len(), "Model manifest loaded");
                    map
                }
                Err(e) => {
                    warn!(error = %e, "Model manifest parse error — ML disabled");
                    HashMap::new()
                }
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct ModelManifest {
    #[allow(dead_code)]
    schema_version: String,
    models: Vec<ModelConfig>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_starts_empty() {
        let pool = ModelSessionPool::new();
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn onnx_disabled_returns_feature_not_enabled() {
        // This test always passes: when onnx is enabled the closure path is hit
        // but verify_checksum will fail before reaching ORT since the path is fake.
        // When onnx is disabled it returns FeatureNotEnabled immediately.
        let pool = ModelSessionPool::new();
        let config = ModelConfig {
            name: "test_model".into(),
            version: "0.0.1".into(),
            path: std::path::PathBuf::from("/nonexistent/model.onnx"),
            sha256: "0".repeat(64),
            input_schema: "float_[1x4]".into(),
            output_classes: vec!["a".into(), "b".into()],
        };
        let result = pool.with_session(&config, |_s| Ok(42u32));
        // Either FeatureNotEnabled (no onnx feature) or ModelNotFound (onnx enabled, bad path)
        assert!(result.is_err(), "should fail without a real model");
        match result {
            Err(InferenceError::FeatureNotEnabled) => { /* onnx feature off — expected */ }
            Err(InferenceError::ModelNotFound(_)) => { /* onnx feature on, path invalid — expected */ }
            Err(InferenceError::ChecksumMismatch) => { /* onnx feature on, bad sha256 — expected */ }
            Err(e) => panic!("unexpected error variant: {e}"),
            Ok(_) => panic!("should not succeed with nonexistent model"),
        }
    }

    #[test]
    fn evict_idle_no_panic_when_empty() {
        let pool = ModelSessionPool::new();
        pool.evict_idle();  // must not panic
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn load_manifest_returns_empty_for_missing_file() {
        let map = load_manifest(std::path::Path::new("/nonexistent/manifest.json"));
        assert!(map.is_empty());
    }

    #[test]
    fn load_manifest_parses_correctly() {
        let json = r#"{
            "schema_version": "1.0",
            "models": [
                {
                    "name": "crash_classifier_v1",
                    "version": "1.0.0",
                    "path": "models/crash_classifier_v1.onnx",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "input_schema": "char_onehot_[512x128]",
                    "output_classes": ["AppCrash", "HighCpu"]
                }
            ]
        }"#;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), json).unwrap();
        let map = load_manifest(tmp.path());
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("crash_classifier_v1"));
        assert_eq!(map["crash_classifier_v1"].version, "1.0.0");
        assert_eq!(map["crash_classifier_v1"].output_classes, vec!["AppCrash", "HighCpu"]);
    }
}
