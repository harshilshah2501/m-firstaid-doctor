mod schema;
pub use schema::*;

use collectors::{RawSnapshot, RawDiskType, RawProcessStatus};
use chrono::Utc;
use uuid::Uuid;

/// Convert a `RawSnapshot` into a `NormalizedSignalSnapshot`.
/// This is pure, deterministic, and infallible — missing fields become None.
pub fn normalize(raw: &RawSnapshot) -> NormalizedSignalSnapshot {
    let top_cpu_process = raw.processes.first().map(|p| NormalizedProcessRef {
        pid: p.pid,
        name: p.name.clone(),
        cpu_pct: p.cpu_pct,
        rss_mb: p.rss_bytes / (1024 * 1024),
        is_system: is_system_process(&p.name),
    });

    let top_mem_process = raw.processes.iter().max_by(|a, b| {
        a.rss_bytes.partial_cmp(&b.rss_bytes).unwrap_or(std::cmp::Ordering::Equal)
    }).map(|p| NormalizedProcessRef {
        pid: p.pid,
        name: p.name.clone(),
        cpu_pct: p.cpu_pct,
        rss_mb: p.rss_bytes / (1024 * 1024),
        is_system: is_system_process(&p.name),
    });

    let total_mb = raw.memory.total_bytes / (1024 * 1024);
    let available_mb = raw.memory.available_bytes / (1024 * 1024);
    let used_mb = raw.memory.used_bytes / (1024 * 1024);
    let memory_used_pct = if total_mb > 0 {
        (used_mb as f32 / total_mb as f32) * 100.0
    } else {
        0.0
    };

    let swap_total_mb = raw.memory.swap_total_bytes / (1024 * 1024);
    let swap_used_mb = raw.memory.swap_used_bytes / (1024 * 1024);
    let swap_used_pct = if swap_total_mb > 0 {
        (swap_used_mb as f32 / swap_total_mb as f32) * 100.0
    } else {
        0.0
    };

    // Normalize disks
    let disks: Vec<NormalizedDisk> = raw.disks.iter().map(|d| {
        let free_pct = if d.total_bytes > 0 {
            (d.free_bytes as f32 / d.total_bytes as f32) * 100.0
        } else {
            0.0
        };
        NormalizedDisk {
            mount_point: d.mount_point.clone(),
            is_system: d.mount_point == "/" || d.mount_point.eq_ignore_ascii_case("C:\\"),
            free_pct,
            free_bytes: d.free_bytes,
            total_bytes: d.total_bytes,
            disk_type: d.disk_type.as_ref().map(|t| match t {
                RawDiskType::Ssd => DiskType::SSD,
                RawDiskType::Hdd => DiskType::HDD,
                RawDiskType::Nvme => DiskType::NVMe,
                RawDiskType::Fusion => DiskType::Fusion,
                RawDiskType::Unknown => DiskType::Unknown,
            }),
            smart_status: None, // populated by platform adapters
        }
    }).collect();

    // Find the primary active network interface (first non-loopback with an IP)
    let active_iface = raw.network.interfaces.iter()
        .find(|i| !i.is_loopback && i.is_up);

    let network = NormalizedNetwork {
        reachable: active_iface.is_some(),
        ipv4: active_iface.and_then(|i| i.ipv4.clone()),
        ipv6: active_iface.and_then(|i| i.ipv6.clone()),
        wifi_ssid: active_iface.and_then(|i| i.wifi_ssid.clone()),
        wifi_signal_dbm: active_iface.and_then(|i| i.wifi_signal_dbm),
        wifi_noise_dbm: active_iface.and_then(|i| i.wifi_noise_dbm),
        dns_servers: raw.network.dns_servers.clone(),
        vpn_active: false,      // platform adapter
        proxy_configured: false, // platform adapter
    };

    let thermal = raw.thermal.as_ref().map(|t| NormalizedThermal {
        state: t.macos_thermal_state.map(|s| match s {
            0 => ThermalState::Normal,
            1 => ThermalState::Fair,
            2 => ThermalState::Serious,
            3 => ThermalState::Critical,
            _ => ThermalState::Normal,
        }),
        cpu_temp_celsius: t.cpu_temp_celsius,
        gpu_temp_celsius: t.gpu_temp_celsius,
        fan_speed_rpm: t.fan_speeds_rpm.first().copied(),
    });

    let battery = raw.battery.as_ref().map(|b| NormalizedBattery {
        present: true,
        charge_pct: b.charge_pct,
        charging: b.is_charging,
        health_pct: b.design_capacity_mah.and_then(|design| {
            b.max_capacity_mah.map(|max| (max as f32 / design as f32) * 100.0)
        }),
        cycle_count: b.cycle_count,
        current_ma: b.current_ma,
        time_remaining_min: b.time_remaining_minutes,
    });

    let memory_pressure = raw.memory.macos_pressure_level.map(|l| match l {
        0 => MemoryPressure::Normal,
        1 => MemoryPressure::Warning,
        2 => MemoryPressure::Critical,
        3 => MemoryPressure::Emergency,
        _ => MemoryPressure::Normal,
    });

    NormalizedSignalSnapshot {
        schema_version: "1.0".into(),
        snapshot_id: Uuid::new_v4(),
        collected_at: raw.collected_at,
        normalized_at: Utc::now(),
        platform: raw.platform.clone(),
        cpu: NormalizedCpu {
            // Single-sample usage; rolling averages computed by feature-extractor over history
            usage_pct: raw.cpu.usage_total_pct,
            core_count: raw.cpu.core_count,
            user_pct: raw.cpu.user_pct,
            system_pct: raw.cpu.system_pct,
            idle_pct: raw.cpu.idle_pct,
            top_process: top_cpu_process,
        },
        memory: NormalizedMemory {
            total_mb,
            available_mb,
            used_pct: memory_used_pct,
            swap_total_mb,
            swap_used_mb,
            swap_used_pct,
            pressure: memory_pressure,
            top_process: top_mem_process,
        },
        disks,
        network,
        thermal,
        battery,
        startup: None,   // populated by platform adapters
        updates: NormalizedUpdates {
            pending_reboot_required: raw.platform_specific.windows
                .as_ref()
                .map(|w| w.pending_reboot_registry)
                .unwrap_or(false),
            recent_update_detected: false,
            update_failed: false,
        },
        crashes: NormalizedCrashes {
            count_1h: 0,
            count_24h: 0,
            count_7d: 0,
            last_process_name: None,
            last_exception_type: None,
        },
        services: Vec::new(),
    }
}

/// Heuristic: is this process a known system process name?
fn is_system_process(name: &str) -> bool {
    const SYSTEM_NAMES: &[&str] = &[
        "kernel_task", "launchd", "WindowServer", "loginwindow", "securityd",
        "trustd", "mds", "mds_stores", "mdworker", "lsass.exe", "csrss.exe",
        "smss.exe", "wininit.exe", "services.exe", "System", "svchost.exe",
    ];
    SYSTEM_NAMES.iter().any(|&n| name.eq_ignore_ascii_case(n))
}
