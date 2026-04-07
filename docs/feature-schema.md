# Normalized Feature Schema

This document defines the complete `FeatureSet` used as input to the rules engine and ML models. Features are derived from a `NormalizedSignalSnapshot`.

## Conventions
- `f32`: floating-point percentage or ratio; range 0.0–100.0 unless noted
- `u64`: bytes, counts, or milliseconds (context-defined)
- `i32`: signed integer
- `Option<T>`: absent on platforms that do not expose this signal
- **Cost**: `cheap` = <1ms, single syscall; `moderate` = <10ms; `expensive` = >50ms, may involve spawning process
- **Freshness**: how stale this field can be before it is considered invalid

---

## CPU Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `cpu_usage_avg_1m` | `f32` | All | cheap | 60s | 1-minute rolling average CPU usage % across all cores |
| `cpu_usage_avg_5m` | `f32` | All | cheap | 300s | 5-minute rolling average CPU usage % |
| `cpu_usage_avg_15m` | `f32` | All | cheap | 900s | 15-minute rolling average |
| `cpu_core_count` | `u32` | All | cheap | static | Logical core count |
| `cpu_user_pct` | `f32` | All | cheap | 30s | % time in user space |
| `cpu_system_pct` | `f32` | All | cheap | 30s | % time in kernel space |
| `cpu_idle_pct` | `f32` | All | cheap | 30s | % idle time |
| `cpu_iowait_pct` | `Option<f32>` | Linux/Win | cheap | 30s | % waiting for I/O (not available on macOS) |
| `cpu_steal_pct` | `Option<f32>` | Linux | cheap | 30s | % stolen by hypervisor |
| `top_process_cpu_pct` | `f32` | All | cheap | 30s | CPU % of top CPU consumer |
| `top_process_name` | `String` | All | cheap | 30s | Name of top CPU consumer |
| `top_process_pid` | `u32` | All | cheap | 30s | PID of top CPU consumer |
| `top_process_is_system` | `bool` | All | cheap | 30s | Whether top process is a system process |
| `top_5_processes_cpu_pct` | `Vec<f32>` | All | cheap | 30s | CPU % of top 5 processes |

---

## Memory Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `memory_total_mb` | `u64` | All | cheap | static | Total physical RAM in MB |
| `memory_used_mb` | `u64` | All | cheap | 30s | Used RAM in MB |
| `memory_available_mb` | `u64` | All | cheap | 30s | Available RAM in MB |
| `memory_used_pct` | `f32` | All | cheap | 30s | Percentage of RAM in use |
| `swap_total_mb` | `u64` | All | cheap | 30s | Total swap space in MB |
| `swap_used_mb` | `u64` | All | cheap | 30s | Used swap in MB |
| `swap_used_pct` | `f32` | All | cheap | 30s | % of swap in use |
| `memory_pressure` | `Option<MemoryPressure>` | macOS | cheap | 10s | macOS: Normal/Warning/Critical/Emergency (IOKit) |
| `page_fault_rate` | `Option<f32>` | All | moderate | 30s | Page faults per second (hard faults) |
| `commit_charge_pct` | `Option<f32>` | Windows | cheap | 30s | Windows: commit charge as % of commit limit |
| `top_process_rss_mb` | `u64` | All | cheap | 30s | RSS (resident set size) of top memory consumer |
| `top_process_mem_name` | `String` | All | cheap | 30s | Name of top memory consumer |
| `top_5_processes_rss_mb` | `Vec<u64>` | All | cheap | 30s | RSS of top 5 memory consumers |

```rust
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
    Emergency,
}
```

---

## Disk Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `disk_mounts` | `Vec<DiskMount>` | All | cheap | 60s | Per-volume disk stats |
| `system_disk_free_pct` | `f32` | All | cheap | 30s | Free % on the system/boot volume |
| `system_disk_free_bytes` | `u64` | All | cheap | 30s | Free bytes on system volume |
| `disk_read_bytes_sec` | `u64` | All | cheap | 30s | Bytes read per second (system-wide) |
| `disk_write_bytes_sec` | `u64` | All | cheap | 30s | Bytes written per second |
| `disk_read_latency_ms` | `Option<f32>` | Win/moderate macOS | moderate | 30s | Average read latency in ms |
| `disk_write_latency_ms` | `Option<f32>` | Win/moderate macOS | moderate | 30s | Average write latency in ms |
| `disk_queue_depth` | `Option<f32>` | Win/Linux | moderate | 30s | Average I/O queue depth |
| `disk_io_util_pct` | `Option<f32>` | Win/Linux | moderate | 30s | % time disk is busy |
| `disk_type` | `Option<DiskType>` | All | cheap | static | SSD / HDD / NVMe / Unknown |
| `smart_status` | `Option<SmartStatus>` | All | expensive | 1h | SMART health: OK / Warning / Failing |
| `top_disk_process_name` | `Option<String>` | All | moderate | 30s | Process with highest disk I/O |
| `top_disk_process_bytes_sec` | `Option<u64>` | All | moderate | 30s | I/O rate of top disk process |

```rust
pub struct DiskMount {
    pub mount_point: String,
    pub device: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub free_pct: f32,
    pub is_system: bool,
}

pub enum DiskType { SSD, HDD, NVMe, Fusion, Unknown }
pub enum SmartStatus { Ok, Warning, Failing, Unknown }
```

---

## Network Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `network_reachable` | `bool` | All | cheap | 15s | Basic internet reachability |
| `default_gateway_ip` | `Option<String>` | All | cheap | 60s | Default gateway IP |
| `default_gateway_ping_ms` | `Option<f32>` | All | moderate | 30s | Ping RTT to default gateway in ms |
| `dns_server_ips` | `Vec<String>` | All | cheap | 60s | Configured DNS servers |
| `dns_resolution_fail_rate` | `f32` | All | moderate | 60s | % of DNS resolution failures over last 10 probes |
| `dns_resolution_latency_ms` | `Option<f32>` | All | moderate | 60s | Average DNS resolution time |
| `packet_loss_pct` | `Option<f32>` | All | expensive | 60s | % packet loss to gateway |
| `wifi_ssid` | `Option<String>` | All | cheap | 30s | Connected WiFi SSID (null if wired) |
| `wifi_signal_dbm` | `Option<i32>` | All | cheap | 15s | WiFi signal strength in dBm |
| `wifi_noise_dbm` | `Option<i32>` | macOS | cheap | 15s | WiFi noise floor in dBm |
| `vpn_active` | `bool` | All | cheap | 15s | Whether VPN tunnel is active |
| `proxy_configured` | `bool` | All | cheap | 60s | Whether HTTP/HTTPS proxy is configured |
| `interface_name` | `String` | All | cheap | 60s | Active network interface name |
| `ipv4_addr` | `Option<String>` | All | cheap | 60s | IPv4 address of active interface |
| `ipv6_addr` | `Option<String>` | All | cheap | 60s | IPv6 address |

---

## Process Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `process_count` | `u32` | All | cheap | 30s | Total running process count |
| `zombie_process_count` | `Option<u32>` | macOS/Linux | cheap | 30s | Zombie (defunct) process count |
| `process_list` | `Vec<ProcessInfo>` | All | moderate | 30s | Snapshot of running processes |

```rust
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f32,
    pub rss_mb: u64,
    pub status: ProcessStatus,
    pub parent_pid: Option<u32>,
    pub is_system: bool,
    pub start_time: DateTime<Utc>,
    pub open_files: Option<u32>,   // expensive, on-demand only
}

pub enum ProcessStatus { Running, Sleeping, Stopped, Zombie, Unknown }
```

---

## Startup / Login Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `startup_items_count` | `u32` | All | moderate | 5m | Total startup/login items count |
| `startup_items` | `Vec<StartupItem>` | All | moderate | 5m | List of startup/login items |
| `launch_agents_count` | `Option<u32>` | macOS | moderate | 5m | macOS: user LaunchAgents count |
| `launch_daemons_count` | `Option<u32>` | macOS | moderate | 5m | macOS: system LaunchDaemons count |
| `login_items_count` | `Option<u32>` | macOS | moderate | 5m | macOS: Login Items (SMLoginItem) count |
| `run_key_items_count` | `Option<u32>` | Windows | moderate | 5m | Windows: HKCU/HKLM Run key entries |
| `task_scheduler_items_count` | `Option<u32>` | Windows | expensive | 15m | Windows: Task Scheduler tasks at login |

```rust
pub struct StartupItem {
    pub name: String,
    pub path: String,
    pub enabled: bool,
    pub source: StartupSource,    // LaunchAgent, LoginItem, RunKey, TaskScheduler
    pub is_system: bool,
    pub vendor: Option<String>,
}
```

---

## App Crash Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `app_crash_count_1h` | `u32` | All | cheap | 5m | App crashes in last 1 hour |
| `app_crash_count_24h` | `u32` | All | cheap | 5m | App crashes in last 24 hours |
| `app_crash_count_7d` | `u32` | All | cheap | 5m | App crashes in last 7 days |
| `crash_process_name` | `Option<String>` | All | cheap | 5m | Most recent crash process name |
| `crash_exception_type` | `Option<String>` | macOS | moderate | 5m | macOS exception type (EXC_BAD_ACCESS etc.) |
| `crash_log_excerpt` | `Option<String>` | All | moderate | 5m | First 2KB of most recent crash log |
| `windows_event_crash_count_24h` | `Option<u32>` | Windows | moderate | 15m | Windows Event Log EventID 1000 count |

---

## Battery / Thermal Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `battery_present` | `bool` | All | cheap | static | Whether battery is present |
| `battery_charge_pct` | `Option<f32>` | All | cheap | 30s | Current battery charge percentage |
| `battery_charging` | `Option<bool>` | All | cheap | 15s | Whether battery is charging |
| `battery_health_pct` | `Option<f32>` | macOS/Win | cheap | 1h | Battery health (design capacity %) |
| `battery_cycle_count` | `Option<u32>` | macOS/Win | cheap | 1h | Battery charge cycle count |
| `battery_current_ma` | `Option<i32>` | macOS | cheap | 30s | Current draw in mA (negative = discharging) |
| `battery_temp_celsius` | `Option<f32>` | macOS | cheap | 30s | Battery temperature |
| `battery_time_remaining_min` | `Option<i32>` | All | cheap | 60s | Minutes remaining (discharging) |
| `cpu_temp_celsius` | `Option<f32>` | macOS (IOKit) | cheap | 15s | CPU die temperature in °C |
| `gpu_temp_celsius` | `Option<f32>` | macOS (IOKit) | cheap | 15s | GPU temperature in °C |
| `fan_speed_rpm` | `Option<u32>` | macOS (IOKit) | cheap | 15s | Primary fan speed |
| `thermal_state` | `Option<ThermalState>` | macOS/Win | cheap | 10s | System thermal state |

```rust
pub enum ThermalState { Normal, Fair, Serious, Critical, Throttling }
```

---

## Update / System State Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `pending_updates_count` | `Option<u32>` | All | expensive | 1h | OS pending updates available |
| `pending_reboot_required` | `bool` | All | cheap | 5m | Reboot required for pending update |
| `reboot_pending_since_hours` | `Option<f32>` | All | cheap | 5m | Hours since reboot became required |
| `last_successful_update_hours_ago` | `Option<f32>` | All | moderate | 1h | Hours since last successful update |
| `update_failed_flag` | `bool` | All | cheap | 5m | Whether last update attempt failed |
| `recent_update_detected` | `bool` | All | cheap | 5m | Update installed within last 24h |

---

## Service Features

| Field | Type | Platform | Cost | Freshness | Description |
|---|---|---|---|---|---|
| `monitored_services` | `Vec<ServiceStatus>` | All | moderate | 30s | Status of configured services |

```rust
pub struct ServiceStatus {
    pub name: String,
    pub display_name: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub restart_count_1h: u32,
    pub uptime_seconds: Option<u64>,
    pub expected_running: bool,   // from config
}
```

---

## Sampling Logic

```
Cadence table:
  - 30-second poll: cpu_*, memory_*, network_reachable, thermal_*, battery_charge_pct
  - 5-minute poll: disk_*, process_list, startup_items, service_status, update_flags
  - 1-hour poll: smart_status, battery_health, pending_updates
  - On-demand: top_disk_process, crash_log_excerpt, detailed process tree
  - Event-driven: crash events (file watcher on /Library/Logs/DiagnosticReports,
                   Windows Event Log subscription)
```

## Field Cost Classification

| Cost | Max time | Examples |
|---|---|---|
| cheap | <1ms | sysinfo cached values, file reads, IOKit property queries |
| moderate | 1–50ms | DNS probe, ping, process I/O accounting, WMI simple queries |
| expensive | 50ms–5s | SMART query, pending updates check, task scheduler enumeration |

Expensive fields are never collected at idle. They are collected on-demand when a trigger fires for the relevant issue class.
