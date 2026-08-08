//! This crate provides a wrapper for Windows variable operations.

mod error;
mod lock;
#[cfg(test)]
mod mock;
mod store;

pub use error::{Error, Result};

use std::io;

use store::{EnvStore, ValueKind, WinRegStore};
use windows::{
    core::{HSTRING, PCWSTR},
    Win32::{
        Foundation::{LPARAM, WPARAM},
        System::Environment::ExpandEnvironmentStringsW,
        UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        },
    },
};

/// A variable name must be non-empty and contain no `;` or NUL.
fn validate_var(var: &str) -> Result<()> {
    if var.is_empty() || var.contains([';', '\0']) {
        Err(Error::InvalidVarName(var.to_owned()))
    } else {
        Ok(())
    }
}

/// A list value must be non-empty and contain no `;` or NUL, since `;` is the
/// list separator.
fn validate_list_value(value: &str) -> Result<()> {
    if value.is_empty() || value.contains([';', '\0']) {
        Err(Error::InvalidListValue(value.to_owned()))
    } else {
        Ok(())
    }
}

/// A scalar value must not contain NUL.
fn validate_scalar_value(value: &str) -> Result<()> {
    if value.contains('\0') {
        Err(Error::InvalidValue)
    } else {
        Ok(())
    }
}

/// Split a `;`-separated list into its non-empty entries.
fn split_list(s: &str) -> Vec<&str> {
    s.split(';').filter(|x| !x.is_empty()).collect()
}

/// Environment variable list entries (usually paths) are case-insensitive on
/// Windows.
fn value_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Expand `%VAR%` placeholders in a string against the current process
/// environment.
fn expand_env_strings(s: &str) -> io::Result<String> {
    let src = HSTRING::from(s);
    unsafe {
        let len = ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), None);
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u16; len as usize];
        let written = ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf));
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        // The returned length includes the terminating NUL.
        buf.truncate(written.saturating_sub(1) as usize);
        Ok(String::from_utf16_lossy(&buf))
    }
}

/// Mirror a stored value into the current process environment, expanding
/// [`ValueKind::ExpandSz`] placeholders first.
fn sync_process_env(var: &str, value: &str, kind: ValueKind) -> Result<()> {
    let expanded = if kind == ValueKind::ExpandSz {
        expand_env_strings(value)?
    } else {
        value.to_owned()
    };
    unsafe { std::env::set_var(var, expanded) };
    Ok(())
}

/// Append a value at the end to the Windows environment variable list
/// (separated by `;`).
///
/// If the value already exists (compared case-insensitively), it will not be
/// added again. The original value type (`REG_SZ` / `REG_EXPAND_SZ`) is
/// preserved.
pub fn append<T1, T2>(var: T1, value: T2) -> Result<()>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    add(var.as_ref(), value.as_ref(), false)
}

/// Prepend a value at the beginning to the Windows environment variable list
/// (separated by `;`).
///
/// If the value already exists (compared case-insensitively), it will not be
/// added again. The original value type (`REG_SZ` / `REG_EXPAND_SZ`) is
/// preserved.
pub fn prepend<T1, T2>(var: T1, value: T2) -> Result<()>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    add(var.as_ref(), value.as_ref(), true)
}

fn add(var: &str, value: &str, front: bool) -> Result<()> {
    let _guard = lock::lock()?;
    let store = WinRegStore::writable()?;
    if let Some((new_value, kind)) = add_inner(&store, var, value, front)? {
        sync_process_env(var, &new_value, kind)?;
        notify_system();
    }
    Ok(())
}

/// Core add logic, generic over the storage backend.
///
/// Returns the written value and its type, or `None` if the value already
/// existed and nothing was written.
fn add_inner<S: EnvStore>(
    store: &S,
    var: &str,
    value: &str,
    front: bool,
) -> Result<Option<(String, ValueKind)>> {
    validate_var(var)?;
    validate_list_value(value)?;
    let (env_var, kind) = store
        .get(var)?
        .unwrap_or_else(|| (String::new(), ValueKind::Sz));
    let mut values = split_list(&env_var);
    if values.iter().any(|v| value_eq(v, value)) {
        return Ok(None);
    }
    if front {
        values.insert(0, value);
    } else {
        values.push(value);
    }
    let new_env_var = values.join(";");
    store.set(var, &new_env_var, kind)?;
    Ok(Some((new_env_var, kind)))
}

/// Remove a value from the Windows environment variable list (separated by
/// `;`).
///
/// # Returns
///
/// - If the value exists and successfully removed, return `Ok(true)`.
/// - If the value does not exist, return `Ok(false)`.
/// - If an error occurred, return `Err(e)`.
pub fn remove_from_list<T1, T2>(var: T1, value: T2) -> Result<bool>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    let var = var.as_ref();
    let _guard = lock::lock()?;
    let store = WinRegStore::writable()?;
    let removed = remove_from_list_inner(&store, var, value.as_ref())?;
    if let Some((new_value, kind)) = &removed {
        sync_process_env(var, new_value, *kind)?;
        notify_system();
    }
    Ok(removed.is_some())
}

/// Core remove logic, generic over the storage backend.
///
/// Returns the written value and its type, or `None` if the value was absent
/// and nothing was written.
fn remove_from_list_inner<S: EnvStore>(
    store: &S,
    var: &str,
    value: &str,
) -> Result<Option<(String, ValueKind)>> {
    validate_var(var)?;
    validate_list_value(value)?;
    let (env_var, kind) = match store.get(var)? {
        Some(t) => t,
        None => return Ok(None),
    };
    let mut values = split_list(&env_var);
    let len = values.len();
    values.retain(|p| !value_eq(p, value));
    if values.len() == len {
        return Ok(None);
    }
    let new_env_var = values.join(";");
    store.set(var, &new_env_var, kind)?;
    Ok(Some((new_env_var, kind)))
}

/// Check if a value exists in the Windows environment variable list (separated
/// by `;`). The comparison is case-insensitive.
pub fn exists_in_list<T1, T2>(var: T1, value: T2) -> Result<bool>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    let store = WinRegStore::read_only()?;
    exists_in_list_inner(&store, var.as_ref(), value.as_ref())
}

fn exists_in_list_inner<S: EnvStore>(store: &S, var: &str, value: &str) -> Result<bool> {
    validate_var(var)?;
    validate_list_value(value)?;
    // Atomic by design: one store read, then an in-memory comparison.
    match store.get(var)? {
        Some((s, _)) => Ok(split_list(&s).iter().any(|p| value_eq(p, value))),
        None => Ok(false),
    }
}

/// Set a var in the Windows environment variable (as `REG_SZ`).
pub fn set<T1: AsRef<str>, T2: AsRef<str>>(var: T1, value: T2) -> Result<()> {
    set_with_kind(var.as_ref(), value.as_ref(), ValueKind::Sz)
}

/// Set a var in the Windows environment variable as `REG_EXPAND_SZ`, so that
/// `%VAR%` placeholders inside the value are expanded by consumers.
pub fn set_expand_string<T1: AsRef<str>, T2: AsRef<str>>(var: T1, value: T2) -> Result<()> {
    set_with_kind(var.as_ref(), value.as_ref(), ValueKind::ExpandSz)
}

fn set_with_kind(var: &str, value: &str, kind: ValueKind) -> Result<()> {
    let _guard = lock::lock()?;
    let store = WinRegStore::writable()?;
    set_inner(&store, var, value, kind)?;
    sync_process_env(var, value, kind)?;
    notify_system();
    Ok(())
}

fn set_inner<S: EnvStore>(store: &S, var: &str, value: &str, kind: ValueKind) -> Result<()> {
    validate_var(var)?;
    validate_scalar_value(value)?;
    store.set(var, value, kind)
}

/// Get a var from the Windows environment variable.
///
/// Returns the raw registry value; `REG_EXPAND_SZ` values are **not**
/// expanded.
pub fn get<T: AsRef<str>>(var: T) -> Result<Option<String>> {
    let var = var.as_ref();
    validate_var(var)?;
    // A single registry read is atomic; no lock is needed here.
    let store = WinRegStore::read_only()?;
    Ok(store.get(var)?.map(|(s, _)| s))
}

/// Remove a var from the Windows environment variable.
pub fn remove<T: AsRef<str>>(var: T) -> Result<()> {
    let var = var.as_ref();
    validate_var(var)?;
    let _guard = lock::lock()?;
    let store = WinRegStore::writable()?;
    store.delete(var)?;
    unsafe { std::env::remove_var(var) };
    notify_system();
    Ok(())
}

fn notify_system() {
    // The HSTRING must outlive the SendMessageTimeoutW call.
    let msg = HSTRING::from("Environment");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(msg.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            500,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockStore;

    const VAR: &str = "WENV-TEST";

    #[test]
    fn test_set_get_delete() -> Result<()> {
        let store = MockStore::new();
        set_inner(&store, VAR, "test", ValueKind::Sz)?;
        assert_eq!(store.get(VAR)?, Some(("test".into(), ValueKind::Sz)));
        set_inner(&store, VAR, "new_test", ValueKind::Sz)?;
        assert_eq!(store.get(VAR)?, Some(("new_test".into(), ValueKind::Sz)));
        store.delete(VAR)?;
        assert_eq!(store.get(VAR)?, None);
        // Deleting an absent value is not an error.
        store.delete(VAR)?;
        Ok(())
    }

    #[test]
    fn test_add_order_and_dedup() -> Result<()> {
        let store = MockStore::new();
        assert!(add_inner(&store, VAR, "a", false)?.is_some());
        assert!(add_inner(&store, VAR, "b", false)?.is_some());
        assert!(add_inner(&store, VAR, "c", true)?.is_some());
        assert_eq!(store.raw(VAR).unwrap().0, "c;a;b");
        // Duplicates are not written again.
        assert!(add_inner(&store, VAR, "a", false)?.is_none());
        assert_eq!(store.raw(VAR).unwrap().0, "c;a;b");
        Ok(())
    }

    #[test]
    fn test_add_preserves_expand_sz() -> Result<()> {
        let store = MockStore::new();
        store.seed(VAR, "%SYSTEMROOT%\\x", ValueKind::ExpandSz);
        let (written, kind) = add_inner(&store, VAR, "y", false)?.unwrap();
        assert_eq!(written, "%SYSTEMROOT%\\x;y");
        assert_eq!(kind, ValueKind::ExpandSz);
        assert_eq!(store.raw(VAR).unwrap().1, ValueKind::ExpandSz);
        Ok(())
    }

    #[test]
    fn test_remove_from_list() -> Result<()> {
        let store = MockStore::new();
        // Absent variable.
        assert!(remove_from_list_inner(&store, VAR, "x")?.is_none());
        store.seed(VAR, "a;b;a", ValueKind::Sz);
        // All occurrences are removed.
        assert!(remove_from_list_inner(&store, VAR, "a")?.is_some());
        assert_eq!(store.raw(VAR).unwrap().0, "b");
        // Absent value: nothing is written.
        assert!(remove_from_list_inner(&store, VAR, "zzz")?.is_none());
        assert_eq!(store.raw(VAR).unwrap().0, "b");
        Ok(())
    }

    #[test]
    fn test_exists_in_list() -> Result<()> {
        let store = MockStore::new();
        assert!(!exists_in_list_inner(&store, VAR, "x")?);
        store.seed(VAR, "C:\\Foo;C:\\Bar", ValueKind::Sz);
        assert!(exists_in_list_inner(&store, VAR, "C:\\Bar")?);
        // Comparison is ASCII case-insensitive.
        assert!(exists_in_list_inner(&store, VAR, "c:\\foo")?);
        assert!(!exists_in_list_inner(&store, VAR, "C:\\Baz")?);
        Ok(())
    }

    #[test]
    fn test_empty_segments_are_dropped_on_rewrite() -> Result<()> {
        let store = MockStore::new();
        store.seed(VAR, "a;;b;", ValueKind::Sz);
        assert!(exists_in_list_inner(&store, VAR, "b")?);
        add_inner(&store, VAR, "c", false)?;
        assert_eq!(store.raw(VAR).unwrap().0, "a;b;c");
        Ok(())
    }

    #[test]
    fn test_invalid_input() {
        let store = MockStore::new();
        assert!(matches!(
            add_inner(&store, VAR, "123;456", false),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            add_inner(&store, VAR, "", false),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            remove_from_list_inner(&store, VAR, "123;456"),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            exists_in_list_inner(&store, VAR, "a\0b"),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            set_inner(&store, "", "test", ValueKind::Sz),
            Err(Error::InvalidVarName(_))
        ));
        assert!(matches!(
            set_inner(&store, "A;B", "test", ValueKind::Sz),
            Err(Error::InvalidVarName(_))
        ));
        assert!(matches!(
            set_inner(&store, VAR, "a\0b", ValueKind::Sz),
            Err(Error::InvalidValue)
        ));
    }

    #[test]
    fn test_error_converts_to_io_error() {
        // An inner io::Error must be extracted without double wrapping, so
        // that callers using `io::Result` keep the original error kind.
        let inner = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let err: io::Error = Error::Registry(inner).into();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

        let err: io::Error = Error::InvalidVarName("".into()).into();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let err: io::Error = Error::UnsupportedValueType("REG_DWORD".into()).into();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// Integration test against the real registry: covers the WinRegStore
    /// boundary (raw value read/write, type round-trip) and
    /// ExpandEnvironmentStringsW, which the mock cannot exercise.
    #[test]
    fn test_integration_expand_sz_roundtrip() -> Result<()> {
        const BASE: &str = "WENV-ITEST-BASE";
        const ENV_VAR: &str = "WENV-ITEST-EXPAND-SZ";
        set(BASE, "hello")?;
        set_expand_string(ENV_VAR, "%WENV-ITEST-BASE%-world")?;

        // get returns the raw, unexpanded value.
        assert_eq!(get(ENV_VAR)?.unwrap(), "%WENV-ITEST-BASE%-world");
        // The current process sees the expanded value.
        assert_eq!(std::env::var(ENV_VAR).unwrap(), "hello-world");

        // List operations must keep the REG_EXPAND_SZ type.
        append(ENV_VAR, "tail")?;
        let store = WinRegStore::read_only()?;
        let (_, kind) = store.get(ENV_VAR)?.unwrap();
        assert_eq!(kind, ValueKind::ExpandSz);
        assert_eq!(get(ENV_VAR)?.unwrap(), "%WENV-ITEST-BASE%-world;tail");
        assert_eq!(std::env::var(ENV_VAR).unwrap(), "hello-world;tail");

        remove(ENV_VAR)?;
        remove(BASE)?;
        assert_eq!(
            std::env::var(ENV_VAR),
            Err(std::env::VarError::NotPresent)
        );
        Ok(())
    }
}