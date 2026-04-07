//! macOS startup item enumeration via launchctl and Login Items.

use collectors::CollectorError;
use std::path::PathBuf;

/// Returns (launch_agents_count, launch_daemons_count, login_items_count)
pub async fn count_launch_agents() -> Result<(u32, u32, u32), CollectorError> {
    let home = std::env::var("HOME").unwrap_or_default();

    let user_agents = count_plist_files(&PathBuf::from(&home).join("Library/LaunchAgents"));
    let system_agents = count_plist_files(&PathBuf::from("/Library/LaunchAgents"));
    let daemons = count_plist_files(&PathBuf::from("/Library/LaunchDaemons"));

    Ok((user_agents + system_agents, daemons, 0 /* login items: Phase 2 via SMLoginItemSetEnabled */))
}

fn count_plist_files(dir: &PathBuf) -> u32 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().extension()
                        .map(|ext| ext == "plist")
                        .unwrap_or(false)
                })
                .count() as u32
        })
        .unwrap_or(0)
}
