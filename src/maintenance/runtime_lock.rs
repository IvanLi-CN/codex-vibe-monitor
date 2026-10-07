use super::*;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

/// Lock both physical database paths, so aliases or a partially shared data directory
/// cannot establish independent writers. Lock files are stable and never unlinked.
pub(crate) struct MaintenanceRuntimeLock {
    files: Vec<File>,
    database_pair_id: String,
    pair_lock_count: usize,
    database_paths: Vec<PathBuf>,
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
            let mut path_hash = Sha256::new();
            for path in &paths {
                use std::os::unix::ffi::OsStrExt;
                let bytes = path.as_os_str().as_bytes();
                path_hash.update((bytes.len() as u64).to_be_bytes());
                path_hash.update(bytes);
            }
            let database_pair_id = format!("{:x}", path_hash.finalize());
            let ready_role = format!("service:ownership-v1:ready:{database_pair_id}");
            let path_pair_lock_name = std::env::temp_dir().join(format!(
                "codex-vibe-monitor-runtime-{database_pair_id}.lock"
            ));
            let mut files = Vec::new();
            let mut busy_roles = Vec::new();
            let mut lock_paths = paths.clone();
            lock_paths.push(path_pair_lock_name);
            let mut inode_ids = Vec::new();
            let mut all_exist = true;
            for path in &paths {
                use std::os::unix::fs::MetadataExt;
                if let Ok(metadata) = std::fs::metadata(path) {
                    inode_ids.push((metadata.dev(), metadata.ino()));
                } else {
                    all_exist = false;
                }
            }
            inode_ids.sort_unstable();
            let mut inode_hash = Sha256::new();
            for (device, inode) in inode_ids {
                inode_hash.update(device.to_be_bytes());
                inode_hash.update(inode.to_be_bytes());
            }
            let inode_pair_id = if all_exist {
                Some(format!("{:x}", inode_hash.finalize()))
            } else {
                None
            };
            if let Some(inode_pair_id) = inode_pair_id.as_deref() {
                let inode_pair_lock = std::env::temp_dir().join(format!(
                    "codex-vibe-monitor-runtime-inode-{inode_pair_id}.lock"
                ));
                lock_paths.push(inode_pair_lock);
            }
            let inode_ready_role = inode_pair_id
                .as_deref()
                .map(|id| format!("service:ownership-v1:ready:{id}"));
            let pair_lock_count = lock_paths.len() - paths.len();
            for path in lock_paths {
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
                return Ok(MaintenanceRuntimeRoute::Offline(Self {
                    files,
                    database_pair_id,
                    pair_lock_count,
                    database_paths: paths,
                }));
            }
            // A partial acquisition, initialization, or another offline command is ambiguous.
            // Do not open/recover either database under that condition.
            if allow_online
                && busy_roles
                    .iter()
                    .any(|role| role == "service:ownership-v1:ready")
                && busy_roles.iter().all(|role| {
                    role == "service:ownership-v1:ready"
                        || role == &ready_role
                        || inode_ready_role.as_deref() == Some(role)
                })
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
        // Readiness belongs to the database pair, not merely to two busy files.
        let path_role = format!("{role}:{}", self.database_pair_id);
        let path_file_count = self.files.len() - self.pair_lock_count;
        for file in &mut self.files[..path_file_count] {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(path_role.as_bytes())?;
            file.sync_data()?;
        }
        for file in &mut self.files[path_file_count..] {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(b"service:ownership-v1:ready")?;
            file.sync_data()?;
        }
        Ok(())
    }

    pub(crate) fn refresh_inode_pair_lock(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::MetadataExt;
            if self.pair_lock_count > 1 {
                return Ok(());
            }
            let mut inode_hash = Sha256::new();
            for path in &self.database_paths {
                let metadata = std::fs::metadata(path)
                    .with_context(|| format!("database path is not ready: {}", path.display()))?;
                inode_hash.update(metadata.dev().to_be_bytes());
                inode_hash.update(metadata.ino().to_be_bytes());
            }
            let inode_pair_id = format!("{:x}", inode_hash.finalize());
            let lock_path = std::env::temp_dir().join(format!(
                "codex-vibe-monitor-runtime-inode-{inode_pair_id}.lock"
            ));
            let mut name = lock_path.as_os_str().to_os_string();
            name.push(".runtime.lock");
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(PathBuf::from(name))?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(b"service:ownership-v1:ready")?;
            file.sync_data()?;
            self.files.push(file);
            self.pair_lock_count += 1;
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
