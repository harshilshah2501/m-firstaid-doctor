pub mod error;
pub mod types;
pub mod cpu;
pub mod memory;
pub mod disk;
pub mod network;
pub mod processes;
pub mod battery;

pub use types::*;
pub use error::CollectorError;

use tokio::time::Instant;
use tracing::{debug, warn};

/// Coordinator that runs all cheap collectors in parallel and assembles a RawSnapshot.
pub struct CollectorSet {
    platform: Platform,
}

impl CollectorSet {
    pub fn new(platform: Platform) -> Self {
        Self { platform }
    }

    /// Run a full collection cycle. Returns a RawSnapshot even if some collectors fail.
    /// Individual collector errors are captured in `collector_errors`.
    pub async fn collect_all(&self) -> RawSnapshot {
        let started = Instant::now();
        debug!("Starting collection cycle");

        let (cpu_result, memory_result, disk_result, network_result, process_result) = tokio::join!(
            cpu::collect(),
            memory::collect(),
            disk::collect(),
            network::collect(),
            processes::collect(),
        );

        let mut errors = Vec::new();

        let cpu = cpu_result.unwrap_or_else(|e| {
            warn!(collector = "cpu", error = %e, "CPU collection failed");
            errors.push(CollectorErrorEntry { collector: "cpu".into(), error: e.to_string() });
            CpuRaw::default()
        });

        let memory = memory_result.unwrap_or_else(|e| {
            warn!(collector = "memory", error = %e, "Memory collection failed");
            errors.push(CollectorErrorEntry { collector: "memory".into(), error: e.to_string() });
            MemoryRaw::default()
        });

        let disks = disk_result.unwrap_or_else(|e| {
            warn!(collector = "disk", error = %e, "Disk collection failed");
            errors.push(CollectorErrorEntry { collector: "disk".into(), error: e.to_string() });
            Vec::new()
        });

        let network = network_result.unwrap_or_else(|e| {
            warn!(collector = "network", error = %e, "Network collection failed");
            errors.push(CollectorErrorEntry { collector: "network".into(), error: e.to_string() });
            NetworkRaw::default()
        });

        let processes = process_result.unwrap_or_else(|e| {
            warn!(collector = "processes", error = %e, "Process collection failed");
            errors.push(CollectorErrorEntry { collector: "processes".into(), error: e.to_string() });
            Vec::new()
        });

        debug!(elapsed_ms = started.elapsed().as_millis(), "Collection cycle complete");

        RawSnapshot {
            schema_version: "1.0".into(),
            collected_at: chrono::Utc::now(),
            platform: self.platform.clone(),
            collector_errors: errors,
            cpu,
            memory,
            disks,
            network,
            processes,
            battery: None,           // populated by platform adapters
            thermal: None,           // populated by platform adapters
            platform_specific: PlatformSpecificRaw::default(),
        }
    }
}
