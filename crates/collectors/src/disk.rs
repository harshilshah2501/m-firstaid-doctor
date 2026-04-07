use crate::{CollectorError, DiskRaw, RawDiskType};
use sysinfo::{Disks, DiskKind};

pub async fn collect() -> Result<Vec<DiskRaw>, CollectorError> {
    tokio::task::spawn_blocking(|| {
        let disks = Disks::new_with_refreshed_list();
        let result = disks
            .iter()
            .map(|d| DiskRaw {
                mount_point: d.mount_point().to_string_lossy().into_owned(),
                device: d.name().to_string_lossy().into_owned(),
                fs_type: d.file_system().to_string_lossy().into_owned(),
                total_bytes: d.total_space(),
                free_bytes: d.available_space(),
                // sysinfo does not expose cumulative I/O counters — these are 0 as baseline.
                // Platform adapters (collectors-mac, collectors-win) fill these in.
                read_bytes_total: 0,
                write_bytes_total: 0,
                is_removable: d.is_removable(),
                disk_type: Some(match d.kind() {
                    DiskKind::SSD => RawDiskType::Ssd,
                    DiskKind::HDD => RawDiskType::Hdd,
                    DiskKind::Unknown(_) => RawDiskType::Unknown,
                }),
            })
            .collect();
        Ok(result)
    })
    .await
    .map_err(|e| CollectorError::OsApi(e.to_string()))?
}
