//! D-Bus bus mode and daemon data path resolution.

use std::path::PathBuf;

use wellbeing_core::Uid;

/// Which D-Bus bus to connect to.
#[derive(Debug, Clone, Copy)]
pub enum BusMode {
    System,
    Session,
}

/// Where the daemon stores its data and operates.
#[derive(Debug, Clone)]
pub enum DaemonMode {
    System { db_path: PathBuf },
    Session { db_path: PathBuf, _uid: Uid },
}

/// Resolve the daemon mode based on effective UID.
pub fn resolve_daemon_mode() -> DaemonMode {
    let euid = nix::unistd::Uid::effective();
    if euid.is_root() {
        DaemonMode::System {
            db_path: PathBuf::from("/var/lib/digital-wellbeing/db.sqlite"),
        }
    } else {
        let xdg_data = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            format!("{home}/.local/share")
        });
        DaemonMode::Session {
            db_path: PathBuf::from(xdg_data).join("digital-wellbeing/db.sqlite"),
            _uid: Uid(euid.as_raw()),
        }
    }
}

/// Determine which bus to use based on the daemon mode.
pub fn resolve_bus(mode: &DaemonMode) -> BusMode {
    match mode {
        DaemonMode::System { .. } => BusMode::System,
        DaemonMode::Session { .. } => BusMode::Session,
    }
}
