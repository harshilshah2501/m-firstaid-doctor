#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn firstaid_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "firstaid.exe"
    } else {
        "firstaid"
    }
}

fn firstaid_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("FIRSTAID_EXE") {
        out.push(PathBuf::from(p));
    }
    let root = repo_root();
    out.push(root.join("target").join("debug").join(firstaid_filename()));
    out.push(root.join("target").join("release").join(firstaid_filename()));
    #[cfg(target_os = "windows")]
    {
        out.push(root.join("target").join("x86_64-pc-windows-msvc").join("release").join("firstaid.exe"));
    }
    #[cfg(target_os = "macos")]
    {
        out.push(
            root.join("target")
                .join("aarch64-apple-darwin")
                .join("release")
                .join("firstaid"),
        );
        out.push(
            root.join("target")
                .join("x86_64-apple-darwin")
                .join("release")
                .join("firstaid"),
        );
    }
    out
}

fn resolve_firstaid() -> PathBuf {
    for p in firstaid_candidates() {
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("firstaid")
}

fn run_firstaid_ui_bundle(exe: &Path) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(["diagnose", "--ui-bundle"]);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn {}: {e}", exe.display()))?;

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "firstaid exited with {}.\n{}",
            output.status,
            if stderr.is_empty() {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            } else {
                stderr
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err("firstaid produced no stdout (expected JSON).".into());
    }
    Ok(stdout)
}

#[tauri::command]
fn run_diagnosis() -> Result<String, String> {
    let exe = resolve_firstaid();
    run_firstaid_ui_bundle(&exe)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![run_diagnosis])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
