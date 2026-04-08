use crate::{CollectorError, ProcessRaw, RawProcessStatus};
use sysinfo::{ProcessStatus, System, ProcessRefreshKind, UpdateKind};
use chrono::{DateTime, Utc};

/// Maximum number of processes to return (sorted by CPU% desc).
/// Prevents excessive memory use on systems with thousands of processes.
const MAX_PROCESSES: usize = 256;

pub async fn collect() -> Result<Vec<ProcessRaw>, CollectorError> {
    tokio::task::spawn_blocking(|| {
        let refresh_kind = ProcessRefreshKind::new()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet);
        let mut sys = System::new();
        sys.refresh_processes_specifics(refresh_kind);

        let mut procs: Vec<ProcessRaw> = sys
            .processes()
            .values()
            .map(|p| {
                let start = p.start_time();
                let start_dt: Option<DateTime<Utc>> = if start > 0 {
                    DateTime::from_timestamp(start as i64, 0)
                } else {
                    None
                };

                ProcessRaw {
                    pid: p.pid().as_u32(),
                    name: p.name().to_string(),
                    cpu_pct: p.cpu_usage(),
                    rss_bytes: p.memory(),
                    vms_bytes: p.virtual_memory(),
                    status: map_status(p.status()),
                    parent_pid: p.parent().map(|pp| pp.as_u32()),
                    start_time: start_dt,
                    exe_path: p.exe().map(|e| e.to_string_lossy().into_owned()),
                }
            })
            .collect();

        // Sort by CPU descending so top consumers are at the front.
        procs.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal));
        procs.truncate(MAX_PROCESSES);

        Ok(procs)
    })
    .await
    .map_err(|e| CollectorError::OsApi(e.to_string()))?
}

fn map_status(status: ProcessStatus) -> RawProcessStatus {
    match status {
        ProcessStatus::Run => RawProcessStatus::Running,
        ProcessStatus::Sleep | ProcessStatus::Idle => RawProcessStatus::Sleeping,
        ProcessStatus::Stop => RawProcessStatus::Stopped,
        ProcessStatus::Zombie => RawProcessStatus::Zombie,
        _ => RawProcessStatus::Unknown,
    }
}
