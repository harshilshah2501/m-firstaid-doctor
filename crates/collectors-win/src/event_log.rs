//! Windows Event Log collection for crash events.
//! Queries Application log for EventID 1000 (application crash) in the past 24 hours.

use collectors::CollectorError;

pub async fn count_app_crashes_24h() -> Result<u32, CollectorError> {
    let output = tokio::process::Command::new("powershell")
        .args([
            "-NonInteractive", "-NoProfile", "-Command",
            r#"
            $since = (Get-Date).AddHours(-24)
            $events = Get-WinEvent -FilterHashtable @{
                LogName = 'Application'
                Id = 1000
                StartTime = $since
            } -ErrorAction SilentlyContinue
            if ($events) { $events.Count } else { 0 }
            "#
        ])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    count_str.parse::<u32>().map_err(|e| CollectorError::Parse(e.to_string()))
}
