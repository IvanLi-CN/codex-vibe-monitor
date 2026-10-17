use super::*;
use std::fs::File;

#[cfg(unix)]
use std::os::fd::AsRawFd;

/// The directory lock fences creation against scavenging. The sidecar owner lock then
/// stays with the task without locking the SQLite work file itself.
pub(crate) struct TaskWorkOwner {
    file: Option<File>,
    path: PathBuf,
}

impl Drop for TaskWorkOwner {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn create_task_work_file(final_path: &Path, work_path: &Path) -> Result<TaskWorkOwner> {
    let _directory_lock = super::super::retention::retention_task_work_directory_lock(final_path)?;
    let _work_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(work_path)
        .context("create owned task-local archive work")?;
    let owner_path = task_work_owner_path(work_path);
    let owner_file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&owner_path)
    {
        Ok(file) => file,
        Err(error) => {
            drop(_work_file);
            let _ = fs::remove_file(work_path);
            return Err(error).context("create task-local archive work owner");
        }
    };
    #[cfg(unix)]
    match try_lock_task_work(&owner_file) {
        Ok(true) => {}
        result => {
            drop(owner_file);
            drop(_work_file);
            let _ = fs::remove_file(&owner_path);
            let _ = fs::remove_file(work_path);
            result?;
            bail!("new task-local archive work unexpectedly has another owner");
        }
    }
    Ok(TaskWorkOwner {
        file: Some(owner_file),
        path: owner_path,
    })
}

fn task_work_owner_path(work_path: &Path) -> PathBuf {
    work_path.with_extension("owner")
}

pub(crate) fn create_retention_identity_work_file(
    final_path: &Path,
    work_path: &Path,
) -> Result<TaskWorkOwner> {
    create_task_work_file(final_path, work_path)
}

pub(super) fn discard_abandoned_task_work(final_path: &Path) -> Result<()> {
    let _directory_lock = super::super::retention::retention_task_work_directory_lock(final_path)?;
    #[cfg(unix)]
    discard_abandoned_task_work_locked(final_path)?;
    Ok(())
}

#[cfg(unix)]
fn try_lock_task_work(file: &File) -> Result<bool> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EAGAIN) || error.raw_os_error() == Some(libc::EWOULDBLOCK)
    {
        return Ok(false);
    }
    Err(error).context("inspect task-local archive work owner")
}

#[cfg(unix)]
fn discard_abandoned_task_work_locked(final_path: &Path) -> Result<()> {
    let Some(parent) = final_path.parent() else {
        return Ok(());
    };
    let Some(name) = final_path.file_name().and_then(|value| value.to_str()) else {
        return Ok(());
    };
    let prefix = format!("{name}.task-");
    let mut stems = std::collections::BTreeSet::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(&prefix)) else {
            continue;
        };
        let Some(stem) = [
            ".sqlite-journal",
            ".sqlite-wal",
            ".sqlite-shm",
            ".sqlite",
            ".tmp",
            ".owner",
        ]
        .iter()
        .find_map(|extension| suffix.strip_suffix(extension)) else {
            continue;
        };
        let parts: Vec<_> = stem.split('-').collect();
        if parts.len() == 3
            && parts[0].parse::<u32>().is_ok_and(|pid| pid > 0)
            && parts[1].parse::<u64>().is_ok()
            && parts[2].parse::<u64>().is_ok()
        {
            stems.insert(format!("{prefix}{stem}"));
        }
    }
    for stem in stems {
        if super::super::retention::retention_run_budget_expired() {
            return Err(super::super::retention::retention_write_deferred(
                "archive_work_cleanup",
            ));
        }
        let owner_path = parent.join(format!("{stem}.owner"));
        let owner_file = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&owner_path)
        {
            Ok(file) => {
                if !try_lock_task_work(&file)? {
                    continue;
                }
                Some(file)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if owner_file.is_none() {
            let work_path = parent.join(format!("{stem}.sqlite"));
            match fs::symlink_metadata(&work_path) {
                Ok(metadata) if !metadata.is_file() => continue,
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
            // Legacy work files used the SQLite path itself as the owner lock. Keep
            // recognizing that format while new work uses a sidecar lock so SQLite can
            // acquire its own database locks on platforms where flock and SQLite overlap.
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&work_path)?;
            if !try_lock_task_work(&file)? {
                continue;
            }
        }
        for extension in [
            ".tmp",
            ".sqlite-journal",
            ".sqlite-wal",
            ".sqlite-shm",
            ".sqlite",
            ".owner",
        ] {
            let path = parent.join(format!("{stem}{extension}"));
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    fs::remove_file(path).context("discard abandoned task-local archive work")?;
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
        }
    }
    Ok(())
}
