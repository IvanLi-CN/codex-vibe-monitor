use super::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

/// Lock both physical database paths, so aliases or a partially shared data directory
/// cannot establish independent writers. Lock files are stable and never unlinked.
pub(crate) struct MaintenanceRuntimeLock {
    files: Vec<File>,
}

pub(crate) enum MaintenanceRuntimeRoute {
    Offline(MaintenanceRuntimeLock),
    Online,
}

fn normalized_database_path(path: &Path) -> Result<PathBuf> {
    ensure_db_directory(path)?;
    if path.exists() {
        return path
            .canonicalize()
            .context("cannot normalize database path");
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent
        .canonicalize()?
        .join(path.file_name().context("database path has no filename")?))
}

impl MaintenanceRuntimeLock {
    pub(crate) fn route(config: &AppConfig, allow_online: bool) -> Result<MaintenanceRuntimeRoute> {
        #[cfg(not(unix))]
        {
            let _ = (config, allow_online);
            bail!("maintenance runtime locking is unavailable on this platform");
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let mut paths = vec![
                normalized_database_path(&config.database_path)?,
                normalized_database_path(&config.maintenance_database_path())?,
            ];
            paths.sort();
            paths.dedup();
            let mut files = Vec::new();
            let mut busy_roles = Vec::new();
            for path in paths {
                let mut name = path.as_os_str().to_os_string();
                name.push(".runtime.lock");
                let mut file = OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(PathBuf::from(name))?;
                // SAFETY: flock receives an owned, live file descriptor and valid operation flags.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    files.push(file);
                } else {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::WouldBlock {
                        return Err(error.into());
                    }
                    let mut role = String::new();
                    file.read_to_string(&mut role)?;
                    busy_roles.push(role);
                }
            }
            if busy_roles.is_empty() {
                return Ok(MaintenanceRuntimeRoute::Offline(Self { files }));
            }
            // A partial acquisition, initialization, or another offline command is ambiguous.
            // Do not open/recover either database under that condition.
            if allow_online
                && files.is_empty()
                && busy_roles
                    .iter()
                    .all(|role| role == "service:ownership-v1:ready")
            {
                Ok(MaintenanceRuntimeRoute::Online)
            } else {
                bail!(
                    "maintenance unavailable: runtime is busy or its protocol cannot be confirmed"
                );
            }
        }
    }

    pub(crate) fn publish_role(&mut self, role: &str) -> Result<()> {
        for file in &mut self.files {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(role.as_bytes())?;
            file.sync_data()?;
        }
        Ok(())
    }
}

impl Drop for MaintenanceRuntimeLock {
    fn drop(&mut self) {
        for file in &mut self.files {
            // Leave no ready marker while releasing the lifetime lock. Never unlink it:
            // waiters and future processes must continue locking the same inode.
            let _ = file.set_len(0);
        }
    }
}
