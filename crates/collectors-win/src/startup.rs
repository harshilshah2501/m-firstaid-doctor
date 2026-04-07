//! Windows startup item collection via Registry Run keys.

use collectors::CollectorError;

pub async fn count_run_keys() -> Result<u32, CollectorError> {
    let output = tokio::process::Command::new("powershell")
        .args([
            "-NonInteractive", "-NoProfile", "-Command",
            r#"
            $hkcu = (Get-ItemProperty "HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" -ErrorAction SilentlyContinue).PSObject.Properties | Where-Object { $_.Name -ne 'PSPath' -and $_.Name -ne 'PSParentPath' -and $_.Name -ne 'PSChildName' -and $_.Name -ne 'PSDrive' -and $_.Name -ne 'PSProvider' }
            $hklm = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" -ErrorAction SilentlyContinue).PSObject.Properties | Where-Object { $_.Name -ne 'PSPath' -and $_.Name -ne 'PSParentPath' -and $_.Name -ne 'PSChildName' -and $_.Name -ne 'PSDrive' -and $_.Name -ne 'PSProvider' }
            ($hkcu.Count + $hklm.Count)
            "#
        ])
        .output()
        .await
        .map_err(|e| CollectorError::OsApi(e.to_string()))?;

    let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    count_str.parse::<u32>().unwrap_or(0).into_ok()
}

trait IntoOk<T> {
    fn into_ok(self) -> Result<T, CollectorError>;
}
impl IntoOk<u32> for u32 {
    fn into_ok(self) -> Result<u32, CollectorError> { Ok(self) }
}
