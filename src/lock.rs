//! Cross-process serialization of registry read-modify-write operations.

use std::io;

use windows::{
    core::{HSTRING, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject, INFINITE},
    },
};

/// Session-local named mutex guarding environment variable writes. `Local\`
/// scopes it to the current session, matching the per-session nature of
/// HKCU environment variables.
const MUTEX_NAME: &str = r"Local\windows-env-registry-lock";

/// RAII guard for the named mutex; released (and the handle closed) on drop.
pub struct NamedMutexGuard(HANDLE);

/// Acquire the named mutex, blocking until it becomes available.
pub fn lock() -> io::Result<NamedMutexGuard> {
    let name = HSTRING::from(MUTEX_NAME);
    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))
            .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
        match WaitForSingleObject(handle, INFINITE) {
            // WAIT_ABANDONED means the previous owner terminated while
            // holding the mutex; ownership is still transferred to us.
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(NamedMutexGuard(handle)),
            other => {
                let _ = CloseHandle(handle);
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("WaitForSingleObject returned {other:?}"),
                ))
            }
        }
    }
}

impl Drop for NamedMutexGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}
