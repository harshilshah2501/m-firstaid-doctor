//! macOS crash report reader.
//!
//! Scans `/Library/Logs/DiagnosticReports/` and the user-level
//! `~/Library/Logs/DiagnosticReports/` for the most-recently modified
//! `.crash` or `.ips` file and returns up to the first 512 characters of its
//! text. This snippet is the input for the CharCNN crash classifier.
//!
//! # Why 512 chars?
//! The CharCNN model was trained on 512-char one-hot windows. The crash header
//! (process name, exception type, OS version, thread state) typically fits
//! within the first 512 characters and carries the most diagnostic signal.

use collectors::CollectorError;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Maximum number of characters passed to the classifier.
const CRASH_WINDOW: usize = 512;

/// Collect the text snippet of the most-recent macOS crash report.
///
/// Returns `Ok(None)` when no crash reports exist (clean machine).
/// Returns `Ok(Some(text))` with up to `CRASH_WINDOW` chars on success.
/// Returns `Err` only on genuine I/O failures after the path is found.
pub async fn collect_latest() -> Result<Option<String>, CollectorError> {
    // Search both the system and the current-user DiagnosticReports directories.
    let dirs: Vec<PathBuf> = {
        let mut v = vec![PathBuf::from("/Library/Logs/DiagnosticReports")];
        if let Some(home) = std::env::var_os("HOME") {
            let mut user_dir = PathBuf::from(home);
            user_dir.push("Library/Logs/DiagnosticReports");
            v.push(user_dir);
        }
        v
    };

    let latest = newest_crash_file(&dirs)?;
    let Some(path) = latest else {
        return Ok(None);
    };

    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| CollectorError::OsApi(format!("crash log read '{}': {e}", path.display())))?;

    // Truncate to the first CRASH_WINDOW UTF-8 characters.
    let snippet: String = text.chars().take(CRASH_WINDOW).collect();
    Ok(Some(snippet))
}

/// Walk `dirs` and return the path of the `.crash` / `.ips` file with the
/// most-recent modification time. Returns `Ok(None)` if none found.
fn newest_crash_file(dirs: &[PathBuf]) -> Result<Option<PathBuf>, CollectorError> {
    let mut best: Option<(PathBuf, SystemTime)> = None;

    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let entries = std::fs::read_dir(dir)
            .map_err(|e| CollectorError::OsApi(format!("readdir '{}': {e}", dir.display())))?;

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "crash" && ext != "ips" {
                continue;
            }
            if !is_app_crash(&path) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    let better = best.as_ref().map(|(_, t)| mtime > *t).unwrap_or(true);
                    if better {
                        best = Some((path, mtime));
                    }
                }
            }
        }
    }

    Ok(best.map(|(p, _)| p))
}

/// Exclude system/kernel crash reports; keep only user-space app crashes.
/// System files have names starting with known OS prefixes.
fn is_app_crash(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    // Skip kernel panics and watchdog reports — they need a different classifier
    !name.starts_with("kernel_") && !name.starts_with("Panicked_")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_app_crash_excludes_kernel_panic() {
        let p = PathBuf::from("/tmp/kernel_2024_01_01.crash");
        assert!(!is_app_crash(&p));
    }

    #[test]
    fn is_app_crash_accepts_app_report() {
        let p = PathBuf::from("/tmp/MyApp_2024_01_01.crash");
        assert!(is_app_crash(&p));
    }

    #[test]
    fn newest_crash_file_empty_dirs_returns_none() {
        let dirs = vec![PathBuf::from("/nonexistent_diagnostic_reports_xyzzy")];
        let result = newest_crash_file(&dirs).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn collect_latest_no_crash_dir_returns_none() {
        // Force HOME to a temp dir with no DiagnosticReports
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        // System dir also won't exist in CI; either Ok(None) or Ok(Some)
        let result = collect_latest().await;
        assert!(result.is_ok(), "should not error when dirs are missing");
    }
}
