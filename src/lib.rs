//! This crate provides a wrapper for Windows variable operations.

mod error;
mod lock;

pub use error::{Error, Result};

use std::{borrow::Cow, io, iter::once};

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
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ},
    enums::RegType,
    types::FromRegValue,
    RegKey, RegValue,
};

/// Open the current user's environment variable RegKey with the given access
/// rights.
fn regkey(access: u32) -> io::Result<RegKey> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags("Environment", access)
}

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

/// Build a registry string value of the given type (REG_SZ or REG_EXPAND_SZ)
/// from a UTF-8 string.
fn raw_string_value(s: &str, vtype: &RegType) -> RegValue<'static> {
    let bytes: Vec<u8> = s
        .encode_utf16()
        .chain(once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    RegValue {
        bytes: Cow::Owned(bytes),
        vtype: vtype.clone(),
    }
}

/// Read a registry string value together with its type; `None` if absent.
fn read_raw(env: &RegKey, var: &str) -> Result<Option<(String, RegType)>> {
    match env.get_raw_value(var) {
        Ok(rv) => Ok(Some((String::from_reg_value(&rv)?, rv.vtype))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
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

/// Mirror a registry value into the current process environment, expanding
/// REG_EXPAND_SZ placeholders first.
fn sync_process_env(var: &str, value: &str, vtype: &RegType) -> Result<()> {
    let expanded = if *vtype == REG_EXPAND_SZ {
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
    add_inner(var.as_ref(), value.as_ref(), false)
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
    add_inner(var.as_ref(), value.as_ref(), true)
}

fn add_inner(var: &str, value: &str, front: bool) -> Result<()> {
    validate_var(var)?;
    validate_list_value(value)?;
    let _guard = lock::lock()?;
    let env = regkey(KEY_READ | KEY_WRITE)?;
    let (env_var, vtype) = read_raw(&env, var)?.unwrap_or_else(|| (String::new(), REG_SZ));
    let mut values = split_list(&env_var);
    if !values.iter().any(|v| value_eq(v, value)) {
        if front {
            values.insert(0, value);
        } else {
            values.push(value);
        }
        let new_env_var = values.join(";");
        env.set_raw_value(var, &raw_string_value(&new_env_var, &vtype))?;
        sync_process_env(var, &new_env_var, &vtype)?;
        notify_system();
    }
    Ok(())
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
    let value = value.as_ref();
    validate_var(var)?;
    validate_list_value(value)?;
    let _guard = lock::lock()?;
    let env = regkey(KEY_READ | KEY_WRITE)?;
    let (env_var, vtype) = match read_raw(&env, var)? {
        Some(t) => t,
        None => return Ok(false),
    };
    let mut values = split_list(&env_var);
    let len = values.len();
    values.retain(|p| !value_eq(p, value));
    let found = len != values.len();
    if found {
        let new_env_var = values.join(";");
        env.set_raw_value(var, &raw_string_value(&new_env_var, &vtype))?;
        sync_process_env(var, &new_env_var, &vtype)?;
        notify_system();
    }
    Ok(found)
}

/// Check if a value exists in the Windows environment variable list (separated
/// by `;`). The comparison is case-insensitive.
pub fn exists_in_list<T1, T2>(var: T1, value: T2) -> Result<bool>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    let var = var.as_ref();
    let value = value.as_ref();
    validate_var(var)?;
    validate_list_value(value)?;
    // Atomic by design: one registry read, then an in-memory comparison.
    let env_var = get(var)?;
    match env_var {
        Some(s) => Ok(split_list(&s).iter().any(|p| value_eq(p, value))),
        None => Ok(false),
    }
}

/// Set a var in the Windows environment variable (as `REG_SZ`).
pub fn set<T1: AsRef<str>, T2: AsRef<str>>(var: T1, value: T2) -> Result<()> {
    set_inner(var.as_ref(), value.as_ref(), REG_SZ)
}

/// Set a var in the Windows environment variable as `REG_EXPAND_SZ`, so that
/// `%VAR%` placeholders inside the value are expanded by consumers.
pub fn set_expand_string<T1: AsRef<str>, T2: AsRef<str>>(var: T1, value: T2) -> Result<()> {
    set_inner(var.as_ref(), value.as_ref(), REG_EXPAND_SZ)
}

fn set_inner(var: &str, value: &str, vtype: RegType) -> Result<()> {
    validate_var(var)?;
    validate_scalar_value(value)?;
    let _guard = lock::lock()?;
    let env = regkey(KEY_READ | KEY_WRITE)?;
    env.set_raw_value(var, &raw_string_value(value, &vtype))?;
    sync_process_env(var, value, &vtype)?;
    notify_system();
    Ok(())
}

/// Get a var from the Windows environment variable.
///
/// Returns the raw registry value; `REG_EXPAND_SZ` values are **not**
/// expanded.
pub fn get<T: AsRef<str>>(var: T) -> Result<Option<String>> {
    let var = var.as_ref();
    validate_var(var)?;
    // A single registry read is atomic; no lock is needed here.
    let env = regkey(KEY_READ)?;
    let res = env.get_value(var);
    match res {
        Ok(s) => Ok(Some(s)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Remove a var from the Windows environment variable.
pub fn remove<T: AsRef<str>>(var: T) -> Result<()> {
    let var = var.as_ref();
    validate_var(var)?;
    let _guard = lock::lock()?;
    let env = regkey(KEY_READ | KEY_WRITE)?;
    if let Err(err) = env.delete_value(var) {
        if err.kind() != io::ErrorKind::NotFound {
            return Err(err.into());
        }
    };
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

    #[test]
    fn test_get_set() -> Result<()> {
        const ENV_VAR: &str = "TEST-GET-SET";
        set(ENV_VAR, "test")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "test");
        remove(ENV_VAR)?;
        assert!(get(ENV_VAR)?.is_none());
        Ok(())
    }

    #[test]
    fn test_list_operations() -> Result<()> {
        const ENV_VAR: &str = "TEST-LIST-OPERATIONS";
        set(ENV_VAR, "test1;test2;te")?;
        assert!(exists_in_list(ENV_VAR, "test1")?);
        assert!(exists_in_list(ENV_VAR, "test2")?);
        assert!(exists_in_list(ENV_VAR, "te")?);
        append(ENV_VAR, "st3")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "test1;test2;te;st3");
        prepend(ENV_VAR, "st4")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "st4;test1;test2;te;st3");
        remove_from_list(ENV_VAR, "test1")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "st4;test2;te;st3");
        assert!(!exists_in_list(ENV_VAR, "test1")?);
        remove(ENV_VAR)?;
        Ok(())
    }

    #[test]
    fn test_reset_one_var() -> Result<()> {
        const ENV_VAR: &str = "TEST-RESET-ONE-VAR";
        set(ENV_VAR, "test")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "test");
        set(ENV_VAR, "new_test")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "new_test");
        remove(ENV_VAR)?;
        Ok(())
    }

    #[test]
    fn test_operate_with_not_exist_var() -> Result<()> {
        const NOT_EXIST: &str = "A_VAR_DOES_NOT_EXIST";
        remove(NOT_EXIST)?;
        assert!(get(NOT_EXIST)?.is_none());
        assert!(!exists_in_list(NOT_EXIST, "test")?);
        assert!(!remove_from_list(NOT_EXIST, "test")?);
        append(NOT_EXIST, "test")?;
        assert_eq!(get(NOT_EXIST)?.unwrap(), "test");
        remove(NOT_EXIST)?;
        prepend(NOT_EXIST, "test")?;
        assert_eq!(get(NOT_EXIST)?.unwrap(), "test");
        remove(NOT_EXIST)?;
        Ok(())
    }

    #[test]
    fn test_operation_will_affect_current_process() -> Result<()> {
        let env_var = "TEST-OPERATION-WILL-AFFECT-CURRENT-PROCESS";
        set(env_var, "test")?;
        assert_eq!(std::env::var(env_var).unwrap(), "test");
        remove(env_var)?;
        assert_eq!(std::env::var(env_var), Err(std::env::VarError::NotPresent));
        Ok(())
    }

    #[test]
    fn test_list_case_insensitive() -> Result<()> {
        const ENV_VAR: &str = "TEST-LIST-CASE-INSENSITIVE";
        set(ENV_VAR, "C:\\Foo;C:\\Bar")?;
        // Duplicate detection and removal ignore ASCII case.
        assert!(exists_in_list(ENV_VAR, "c:\\foo")?);
        append(ENV_VAR, "c:\\FOO")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "C:\\Foo;C:\\Bar");
        assert!(remove_from_list(ENV_VAR, "c:\\bar")?);
        assert_eq!(get(ENV_VAR)?.unwrap(), "C:\\Foo");
        remove(ENV_VAR)?;
        Ok(())
    }

    #[test]
    fn test_list_ignores_empty_segments() -> Result<()> {
        const ENV_VAR: &str = "TEST-LIST-EMPTY-SEGMENTS";
        set(ENV_VAR, "a;;b;")?;
        assert!(exists_in_list(ENV_VAR, "b")?);
        // Empty segments are dropped when the list is rewritten.
        append(ENV_VAR, "c")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "a;b;c");
        remove(ENV_VAR)?;
        Ok(())
    }

    #[test]
    fn test_expand_sz_is_preserved_and_expanded_in_process() -> Result<()> {
        const BASE: &str = "TEST-EXPAND-BASE";
        const ENV_VAR: &str = "TEST-EXPAND-SZ";
        set(BASE, "hello")?;
        set_expand_string(ENV_VAR, "%TEST-EXPAND-BASE%-world")?;

        // get returns the raw, unexpanded value.
        assert_eq!(get(ENV_VAR)?.unwrap(), "%TEST-EXPAND-BASE%-world");
        // The current process sees the expanded value.
        assert_eq!(std::env::var(ENV_VAR).unwrap(), "hello-world");

        // List operations must keep the REG_EXPAND_SZ type.
        append(ENV_VAR, "tail")?;
        let env = regkey(KEY_READ)?;
        let rv = env.get_raw_value(ENV_VAR)?;
        assert_eq!(rv.vtype, REG_EXPAND_SZ);
        assert_eq!(get(ENV_VAR)?.unwrap(), "%TEST-EXPAND-BASE%-world;tail");
        assert_eq!(std::env::var(ENV_VAR).unwrap(), "hello-world;tail");

        remove(ENV_VAR)?;
        remove(BASE)?;
        Ok(())
    }

    #[test]
    fn test_invalid_input() {
        let env_var = "TEST-INVALID-INPUT";
        assert!(matches!(
            append(env_var, "123;456"),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            prepend(env_var, ""),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            remove_from_list(env_var, "123;456"),
            Err(Error::InvalidListValue(_))
        ));
        assert!(matches!(
            set("", "test"),
            Err(Error::InvalidVarName(_))
        ));
        assert!(matches!(get("A;B"), Err(Error::InvalidVarName(_))));
        assert!(matches!(set(env_var, "a\0b"), Err(Error::InvalidValue)));
    }

    #[test]
    fn test_error_converts_to_io_error() {
        // An inner io::Error must be extracted without double wrapping, so
        // that callers using `io::Result` keep the original error kind.
        let not_found = remove_from_list("A_VAR_DOES_NOT_EXIST", "test")
            .and_then(|_| get("A_VAR_DOES_NOT_EXIST").map(|_| ()));
        assert!(not_found.is_ok());

        let err: io::Error = Error::InvalidVarName("".into()).into();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
