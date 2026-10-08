use super::RuntimeDatabaseFiles;
use anyhow::{Context, Result, ensure};
use libsqlite3_sys as ffi;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

struct Registration {
    base: usize,
    proof: Weak<RuntimeDatabaseFiles>,
}

static REGISTRATIONS: OnceLock<Mutex<HashMap<usize, Registration>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
static OPEN_REPLACEMENTS: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn replace_during_next_open(path: PathBuf, replacement: PathBuf) {
    OPEN_REPLACEMENTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .insert(path, replacement);
}

fn registration(vfs: *mut ffi::sqlite3_vfs) -> Option<(usize, Arc<RuntimeDatabaseFiles>)> {
    let registrations = REGISTRATIONS.get()?.lock().ok()?;
    let registration = registrations.get(&(vfs as usize))?;
    Some((registration.base, registration.proof.upgrade()?))
}

pub(super) fn register(proof: &Arc<RuntimeDatabaseFiles>) -> Result<String> {
    // SAFETY: SQLite owns its default VFS for the process lifetime. Registration
    // copies its ABI and application data; only the two guarded callbacks change.
    unsafe {
        let base = ffi::sqlite3_vfs_find(std::ptr::null());
        ensure!(!base.is_null(), "SQLite default VFS is unavailable");
        ensure!(
            CStr::from_ptr((*base).zName)
                .to_bytes()
                .starts_with(b"unix"),
            "maintenance database identity checks require the Unix SQLite VFS"
        );
        let name = format!(
            "cvm-owned-runtime-{}",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let mut wrapper = Box::new(*base);
        wrapper.pNext = std::ptr::null_mut();
        wrapper.zName = CString::new(name.clone())?.into_raw();
        wrapper.xOpen = Some(open);
        wrapper.xFullPathname = Some(full_pathname);
        let pointer = Box::into_raw(wrapper);
        REGISTRATIONS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| anyhow::anyhow!("SQLite runtime registration lock poisoned"))?
            .insert(
                pointer as usize,
                Registration {
                    base: base as usize,
                    proof: Arc::downgrade(proof),
                },
            );
        ensure!(
            ffi::sqlite3_vfs_register(pointer, 0) == ffi::SQLITE_OK,
            "cannot register SQLite runtime identity guard"
        );
        // SQLite connections (including ATTACH) retain VFS pointers. Keep this tiny
        // registration process-lived; its weak proof never prolongs database locks.
        Ok(name)
    }
}

unsafe fn filename(name: *const c_char) -> Option<PathBuf> {
    if name.is_null() {
        return None;
    }
    // SAFETY: the SQLite callback supplies a live, NUL-terminated filename.
    let bytes = unsafe { CStr::from_ptr(name) }.to_bytes();
    Some(Path::new(std::ffi::OsStr::from_bytes(bytes)).to_path_buf())
}

unsafe extern "C" fn full_pathname(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    output_len: c_int,
    output: *mut c_char,
) -> c_int {
    let Some((base, proof)) = registration(vfs) else {
        return ffi::SQLITE_CANTOPEN;
    };
    if proof.validate().is_err() {
        return ffi::SQLITE_CANTOPEN;
    }
    // SAFETY: delegate to the original VFS with its own pointer and SQLite's
    // callback arguments; only inspect output after successful initialization.
    unsafe {
        let base = base as *mut ffi::sqlite3_vfs;
        let Some(callback) = (*base).xFullPathname else {
            return ffi::SQLITE_CANTOPEN;
        };
        let rc = callback(base, name, output_len, output);
        if rc != ffi::SQLITE_OK {
            return rc;
        }
        let input = filename(name);
        if proof.validate().is_err()
            || input.as_ref().is_some_and(|path| {
                proof.paths.contains(path) && filename(output).as_ref() != Some(path)
            })
        {
            return ffi::SQLITE_CANTOPEN;
        }
        rc
    }
}

unsafe extern "C" fn open(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    output_flags: *mut c_int,
) -> c_int {
    let Some((base, proof)) = registration(vfs) else {
        return ffi::SQLITE_CANTOPEN;
    };
    if proof.validate().is_err() {
        return ffi::SQLITE_CANTOPEN;
    }
    // SAFETY: SQLite allocates the original VFS's szOsFile; the delegated open
    // initializes its methods. On guard failure close once and clear pMethods.
    unsafe {
        let base = base as *mut ffi::sqlite3_vfs;
        let Some(callback) = (*base).xOpen else {
            return ffi::SQLITE_CANTOPEN;
        };
        // The test seam swaps only this test's file around the native open, then
        // restores the original path so metadata checks alone cannot catch it.
        #[cfg(test)]
        let replacement = filename(name).and_then(|path| {
            OPEN_REPLACEMENTS.get().and_then(|replacements| {
                replacements
                    .lock()
                    .ok()?
                    .remove(&path)
                    .map(|replacement| (path, replacement))
            })
        });
        #[cfg(test)]
        let displaced = replacement
            .as_ref()
            .map(|(path, _)| path.with_extension("open-displaced"));
        #[cfg(test)]
        if let Some(((path, replacement), displaced)) = replacement.as_ref().zip(displaced.as_ref())
        {
            if std::fs::rename(path, displaced)
                .and_then(|()| std::fs::rename(replacement, path))
                .is_err()
            {
                return ffi::SQLITE_CANTOPEN;
            }
        }
        let rc = callback(base, name, file, flags, output_flags);
        #[cfg(test)]
        let restored = replacement.as_ref().zip(displaced.as_ref()).is_none_or(
            |((path, replacement), displaced)| {
                std::fs::rename(path, replacement)
                    .and_then(|()| std::fs::rename(displaced, path))
                    .is_ok()
            },
        );
        if rc != ffi::SQLITE_OK {
            return rc;
        }
        let guarded = flags & ffi::SQLITE_OPEN_MAIN_DB != 0
            && filename(name).is_some_and(|path| proof.paths.contains(&path));
        let valid = if guarded {
            let mut moved: c_int = 1;
            (*(*file).pMethods)
                .xFileControl
                .context("Unix SQLite file identity control unavailable")
                .map(|control| {
                    control(
                        file,
                        ffi::SQLITE_FCNTL_HAS_MOVED,
                        (&mut moved as *mut c_int).cast(),
                    ) == ffi::SQLITE_OK
                        && moved == 0
                })
                .unwrap_or(false)
        } else {
            true
        };
        if !valid || proof.validate().is_err() {
            if let Some(close) = (*(*file).pMethods).xClose {
                close(file);
            }
            (*file).pMethods = std::ptr::null();
            return ffi::SQLITE_CANTOPEN;
        }
        #[cfg(test)]
        if !restored {
            if let Some(close) = (*(*file).pMethods).xClose {
                close(file);
            }
            (*file).pMethods = std::ptr::null();
            return ffi::SQLITE_CANTOPEN;
        }
        rc
    }
}
