use super::*;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(unix)]
mod sqlite_vfs;

struct RuntimeDatabaseFiles {
    paths: Vec<PathBuf>,
    files: Vec<File>,
}

impl RuntimeDatabaseFiles {
    fn validate(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            for (path, file) in self.paths.iter().zip(&self.files) {
                let held = file.metadata()?;
                let current = std::fs::metadata(path)?;
                anyhow::ensure!(
                    held.nlink() == 1
                        && current.nlink() == 1
                        && (held.dev(), held.ino()) == (current.dev(), current.ino()),
                    "maintenance unavailable: database path or journal ownership changed"
                );
            }
        }
        Ok(())
    }
}

/// Lock both physical database paths, so aliases or a partially shared data directory
/// cannot establish independent writers. Lock files are stable and never unlinked.
pub(crate) struct MaintenanceRuntimeLock {
    files: Vec<File>,
    // Database descriptors are lifetime-locked, but never receive protocol markers.
    database_files: Arc<RuntimeDatabaseFiles>,
    database_pair_id: String,
    pair_lock_count: usize,
    inode_pair_id: Option<String>,
    database_paths: Vec<PathBuf>,
    guarded_vfs: Option<String>,
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
            let mut hardlinked_database = false;
            for path in &paths {
                use std::os::unix::fs::MetadataExt;
                if let Ok(metadata) = std::fs::metadata(path) {
                    inode_ids.push((metadata.dev(), metadata.ino()));
                    hardlinked_database |= metadata.nlink() > 1;
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
                // SQLite journals are named after the database path. With multiple hard links,
                // an offline caller cannot establish which name owns crash-recovery state.
                anyhow::ensure!(
                    !hardlinked_database,
                    "maintenance unavailable: hard-linked database journal ownership is ambiguous"
                );
                let mut database_files = Vec::with_capacity(paths.len());
                for path in &paths {
                    let file = OpenOptions::new()
                        .create(true)
                        .truncate(false)
                        .read(true)
                        .write(true)
                        .open(path)?;
                    // Protect individual inodes as well as the pair: a partial hard-link alias
                    // must not establish a second owner with a different companion database.
                    // SAFETY: the owned database descriptor stays alive for the runtime lease.
                    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
                    {
                        return Err(std::io::Error::last_os_error())
                            .context("maintenance unavailable: database inode already owned");
                    }
                    database_files.push(file);
                }
                let mut owner = Self {
                    files,
                    database_files: Arc::new(RuntimeDatabaseFiles {
                        paths: paths.clone(),
                        files: database_files,
                    }),
                    database_pair_id,
                    pair_lock_count,
                    inode_pair_id,
                    database_paths: paths,
                    guarded_vfs: None,
                };
                // Files and all identity locks exist before the caller may initialize SQLite.
                owner.refresh_inode_pair_lock()?;
                return Ok(MaintenanceRuntimeRoute::Offline(owner));
            }
            // A partial acquisition, initialization, or another offline command is ambiguous.
            // Do not open/recover either database under that condition.
            if allow_online
                && all_exist
                && files.is_empty()
                && busy_roles.iter().any(|role| role == &ready_role)
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
        self.database_files.validate()?;
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
            file.write_all(role.as_bytes())?;
            file.sync_data()?;
        }
        Ok(())
    }

    pub(crate) fn refresh_inode_pair_lock(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::MetadataExt;
            // Revalidate even when the inode-pair protocol lock already exists. Its
            // name is no proof that the paths still resolve to our held descriptors.
            self.database_files.validate()?;
            let mut inode_ids = Vec::with_capacity(self.database_paths.len());
            for file in &self.database_files.files {
                let metadata = file.metadata()?;
                inode_ids.push((metadata.dev(), metadata.ino()));
            }
            inode_ids.sort_unstable();
            let mut inode_hash = Sha256::new();
            for (device, inode) in inode_ids {
                inode_hash.update(device.to_be_bytes());
                inode_hash.update(inode.to_be_bytes());
            }
            let inode_pair_id = format!("{:x}", inode_hash.finalize());
            if let Some(acquired) = &self.inode_pair_id {
                anyhow::ensure!(
                    acquired == &inode_pair_id,
                    "maintenance unavailable: database inode changed during lock acquisition"
                );
                return Ok(());
            }
            let lock_path = std::env::temp_dir().join(format!(
                "codex-vibe-monitor-runtime-inode-{inode_pair_id}.lock"
            ));
            let mut name = lock_path.as_os_str().to_os_string();
            name.push(".runtime.lock");
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(PathBuf::from(name))?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            file.set_len(0)?;
            // The new identity lock is unavailable until the caller publishes its role.
            // Acquiring it must not advertise an initializing service or offline CLI as ready.
            file.sync_data()?;
            self.files.push(file);
            self.pair_lock_count += 1;
            self.inode_pair_id = Some(inode_pair_id);
        }
        Ok(())
    }

    pub(crate) fn sqlite_connect_options(
        &mut self,
        path: &Path,
        options: SqliteConnectOptions,
    ) -> Result<SqliteConnectOptions> {
        self.database_files.validate()?;
        let normalized = normalized_database_path(path)?;
        anyhow::ensure!(
            self.database_paths.contains(&normalized),
            "maintenance unavailable: database path is outside the runtime lease"
        );
        #[cfg(unix)]
        {
            if self.guarded_vfs.is_none() {
                self.guarded_vfs = Some(sqlite_vfs::register(&self.database_files)?);
            }
            // Pin the canonical spelling, including for later pooled connections.
            Ok(options
                .filename(normalized)
                .vfs(self.guarded_vfs.clone().expect("registered guarded VFS")))
        }
        #[cfg(not(unix))]
        {
            let _ = options;
            bail!("maintenance runtime locking is unavailable on this platform");
        }
    }
}

#[cfg(all(test, unix))]
pub(crate) use sqlite_vfs::replace_during_next_open;

impl Drop for MaintenanceRuntimeLock {
    fn drop(&mut self) {
        for file in &mut self.files {
            // Leave no ready marker while releasing the lifetime lock. Never unlink it:
            // waiters and future processes must continue locking the same inode.
            let _ = file.set_len(0);
        }
    }
}
