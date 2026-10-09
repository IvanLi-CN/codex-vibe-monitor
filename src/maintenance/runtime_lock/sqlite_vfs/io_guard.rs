use super::*;
use std::ffi::c_void;

#[repr(C)]
pub(super) struct GuardedFile {
    file: ffi::sqlite3_file,
    state: *mut FileState,
}

struct FileState {
    // Preserve the native file layout and pMethods; Unix VFS callbacks must see
    // their own sqlite3_file, never the outer wrapper.
    native: Vec<u128>,
    proof: Weak<RuntimeDatabaseFiles>,
    identity: Option<(PathBuf, (u64, u64))>,
    shm_identity: Option<(PathBuf, (u64, u64))>,
}

impl FileState {
    fn native(&mut self) -> *mut ffi::sqlite3_file {
        self.native.as_mut_ptr().cast()
    }

    fn valid(&self) -> bool {
        self.proof.upgrade().is_some_and(|proof| {
            proof.validate().is_ok()
                && self.identity.as_ref().is_none_or(|(path, identity)| {
                    proof.validate_file_identity(path, Some(*identity)).is_ok()
                })
                && self.shm_identity.as_ref().is_none_or(|(path, identity)| {
                    proof.validate_file_identity(path, Some(*identity)).is_ok()
                })
        })
    }
}

pub(super) unsafe fn wrap_open(
    base: *mut ffi::sqlite3_vfs,
    proof: &Arc<RuntimeDatabaseFiles>,
    name: *const c_char,
    outer: *mut ffi::sqlite3_file,
    flags: c_int,
    output_flags: *mut c_int,
) -> c_int {
    unsafe {
        (*outer).pMethods = std::ptr::null();
        let Some(callback) = (*base).xOpen else {
            return ffi::SQLITE_CANTOPEN;
        };
        let mut state = Box::new(FileState {
            native: vec![0; ((*base).szOsFile as usize).div_ceil(std::mem::size_of::<u128>())],
            proof: Arc::downgrade(proof),
            identity: None,
            shm_identity: None,
        });
        let native = state.native();
        let rc = callback(base, name, native, flags, output_flags);
        if rc != ffi::SQLITE_OK {
            if !(*native).pMethods.is_null()
                && let Some(close) = (*(*native).pMethods).xClose
            {
                close(native);
            }
            return rc;
        }
        if let Some(path) = filename(name) {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = std::fs::metadata(&path) {
                if owned_sidecar(proof, &path) {
                    let Ok(mut sidecars) = proof.sqlite_sidecars.lock() else {
                        if let Some(close) = (*(*native).pMethods).xClose {
                            close(native);
                        }
                        return ffi::SQLITE_CANTOPEN;
                    };
                    sidecars.insert(path.clone(), (metadata.dev(), metadata.ino()));
                }
                state.identity = Some((path, (metadata.dev(), metadata.ino())));
            }
        }
        let wrapper = outer.cast::<GuardedFile>();
        (*wrapper).state = Box::into_raw(state);
        (*outer).pMethods = &METHODS;
        rc
    }
}

unsafe fn state<'a>(file: *mut ffi::sqlite3_file) -> &'a mut FileState {
    unsafe { &mut *(*file.cast::<GuardedFile>()).state }
}

pub(super) unsafe fn native(file: *mut ffi::sqlite3_file) -> *mut ffi::sqlite3_file {
    unsafe { state(file).native() }
}

// SQLite serializes operations on a connection. Each callback borrows its own
// state only for the synchronous native call; no pointer escapes this boundary.
macro_rules! delegate {
    ($name:ident, $method:ident, $guard:expr, ($($arg:ident: $ty:ty),*), $fallback:expr) => {
        unsafe extern "C" fn $name(file: *mut ffi::sqlite3_file, $($arg: $ty),*) -> c_int {
            unsafe {
                let state = state(file);
                if $guard && !state.valid() { return ffi::SQLITE_IOERR; }
                let native = state.native();
                (*(*native).pMethods).$method
                    .map(|callback| callback(native, $($arg),*))
                    .unwrap_or($fallback)
            }
        }
    };
}

delegate!(read, xRead, true, (buffer: *mut c_void, amount: c_int, offset: ffi::sqlite3_int64), ffi::SQLITE_IOERR);
delegate!(write, xWrite, true, (buffer: *const c_void, amount: c_int, offset: ffi::sqlite3_int64), ffi::SQLITE_IOERR);
delegate!(truncate, xTruncate, true, (size: ffi::sqlite3_int64), ffi::SQLITE_IOERR);
delegate!(sync, xSync, true, (flags: c_int), ffi::SQLITE_IOERR);
delegate!(file_size, xFileSize, true, (size: *mut ffi::sqlite3_int64), ffi::SQLITE_IOERR);
delegate!(lock, xLock, true, (level: c_int), ffi::SQLITE_IOERR);
delegate!(unlock, xUnlock, false, (level: c_int), ffi::SQLITE_IOERR);
delegate!(reserved, xCheckReservedLock, true, (result: *mut c_int), ffi::SQLITE_IOERR);
delegate!(control, xFileControl, true, (op: c_int, arg: *mut c_void), ffi::SQLITE_NOTFOUND);
delegate!(sector_size, xSectorSize, false, (), 4096);
delegate!(device, xDeviceCharacteristics, false, (), 0);
delegate!(shm_lock, xShmLock, true, (offset: c_int, count: c_int, flags: c_int), ffi::SQLITE_IOERR);

unsafe extern "C" fn shm_map(
    file: *mut ffi::sqlite3_file,
    page: c_int,
    size: c_int,
    extend: c_int,
    result: *mut *mut c_void,
) -> c_int {
    unsafe {
        let state = state(file);
        if !state.valid() {
            return ffi::SQLITE_IOERR;
        }
        let native = state.native();
        let rc = (*(*native).pMethods)
            .xShmMap
            .map(|map| map(native, page, size, extend, result))
            .unwrap_or(ffi::SQLITE_IOERR);
        if rc == ffi::SQLITE_OK
            && state.shm_identity.is_none()
            && let Some((path, _)) = &state.identity
        {
            use std::os::unix::fs::MetadataExt;
            let mut shm = path.as_os_str().to_os_string();
            shm.push("-shm");
            let shm = PathBuf::from(shm);
            if let Ok(metadata) = std::fs::metadata(&shm) {
                state.shm_identity = Some((shm, (metadata.dev(), metadata.ino())));
            }
        }
        rc
    }
}

unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    unsafe {
        let wrapper = file.cast::<GuardedFile>();
        let mut state = Box::from_raw((*wrapper).state);
        let native = state.native();
        let rc = (*(*native).pMethods)
            .xClose
            .map(|close| close(native))
            .unwrap_or(ffi::SQLITE_IOERR);
        (*file).pMethods = std::ptr::null();
        (*wrapper).state = std::ptr::null_mut();
        rc
    }
}

unsafe extern "C" fn shm_barrier(file: *mut ffi::sqlite3_file) {
    unsafe {
        let native = state(file).native();
        if let Some(callback) = (*(*native).pMethods).xShmBarrier {
            callback(native);
        }
    }
}

unsafe extern "C" fn shm_unmap(file: *mut ffi::sqlite3_file, delete: c_int) -> c_int {
    unsafe {
        let state = state(file);
        // Always permit close/unmap, but do not delete a path after proof loss.
        let delete = if state.valid() { delete } else { 0 };
        let native = state.native();
        let rc = (*(*native).pMethods)
            .xShmUnmap
            .map(|unmap| unmap(native, delete))
            .unwrap_or(ffi::SQLITE_OK);
        if rc == ffi::SQLITE_OK {
            state.shm_identity = None;
        }
        rc
    }
}

unsafe extern "C" fn fetch(
    file: *mut ffi::sqlite3_file,
    _: ffi::sqlite3_int64,
    _: c_int,
    result: *mut *mut c_void,
) -> c_int {
    unsafe {
        if !state(file).valid() {
            return ffi::SQLITE_IOERR;
        }
        // Disable mmap so reads and writes always cross a validated I/O callback.
        *result = std::ptr::null_mut();
        ffi::SQLITE_OK
    }
}

unsafe extern "C" fn unfetch(
    _: *mut ffi::sqlite3_file,
    _: ffi::sqlite3_int64,
    _: *mut c_void,
) -> c_int {
    ffi::SQLITE_OK
}

static METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 3,
    xClose: Some(close),
    xRead: Some(read),
    xWrite: Some(write),
    xTruncate: Some(truncate),
    xSync: Some(sync),
    xFileSize: Some(file_size),
    xLock: Some(lock),
    xUnlock: Some(unlock),
    xCheckReservedLock: Some(reserved),
    xFileControl: Some(control),
    xSectorSize: Some(sector_size),
    xDeviceCharacteristics: Some(device),
    xShmMap: Some(shm_map),
    xShmLock: Some(shm_lock),
    xShmBarrier: Some(shm_barrier),
    xShmUnmap: Some(shm_unmap),
    xFetch: Some(fetch),
    xUnfetch: Some(unfetch),
};
