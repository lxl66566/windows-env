//! This crate provides a wrapper for Windows variable operations.

mod error;

pub use error::{Error, Result};

use std::{io, sync::RwLock};

use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        },
    },
};
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
    RegKey,
};

static LOCK: RwLock<()> = RwLock::new(());

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

/// Append a value at the end to the Windows environment variable list
/// (separated by `;`).
///
/// If the value already exists, it will not be added again.
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
/// If the value already exists, it will not be added again.
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
    let _lock = LOCK.write().unwrap();
    let env = regkey(KEY_READ | KEY_WRITE)?;
    let get_res = env.get_value(var);
    let env_var: String = match get_res {
        Ok(s) => s,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::default(),
        Err(err) => return Err(err.into()),
    };
    let mut values = env_var
        .split(';')
        .filter(|x| !x.is_empty())
        .collect::<Vec<&str>>();
    if !values.contains(&value) {
        if front {
            values.insert(0, value);
        } else {
            values.push(value);
        }
        let new_env_var = values.join(";");
        env.set_value(var, &new_env_var)?;
        unsafe { std::env::set_var(var, &new_env_var) };
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
    let _lock = LOCK.write().unwrap();
    let env = regkey(KEY_READ | KEY_WRITE)?;
    let get_res = env.get_value(var);
    let env_var: String = match get_res {
        Ok(s) => s,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let mut values = env_var.split(';').collect::<Vec<&str>>();
    let len = values.len();
    values.retain(|p| p != &value);
    let found = len != values.len();
    let new_env_var = values.join(";");
    env.set_value(var, &new_env_var)?;
    unsafe { std::env::set_var(var, &new_env_var) };
    notify_system();
    Ok(found)
}

/// Check if a value exists in the Windows environment variable list (separated
/// by `;`).
pub fn exists_in_list<T1, T2>(var: T1, value: T2) -> Result<bool>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    let var = var.as_ref();
    let value = value.as_ref();
    validate_var(var)?;
    validate_list_value(value)?;
    // locked in `get`
    let env_var = get(var)?;
    match env_var {
        Some(s) => Ok(s.split(';').any(|p| p == value)),
        None => Ok(false),
    }
}

/// Set a var in the Windows environment variable.
pub fn set<T1: AsRef<str>, T2: AsRef<str>>(var: T1, value: T2) -> Result<()> {
    let var = var.as_ref();
    let value = value.as_ref();
    validate_var(var)?;
    validate_scalar_value(value)?;
    let _lock = LOCK.write().unwrap();
    let env = regkey(KEY_READ | KEY_WRITE)?;
    env.set_value(var, &value)?;
    unsafe { std::env::set_var(var, value) };
    notify_system();
    Ok(())
}

/// Get a var from the Windows environment variable.
pub fn get<T: AsRef<str>>(var: T) -> Result<Option<String>> {
    let var = var.as_ref();
    validate_var(var)?;
    let _lock = LOCK.read().unwrap();
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
    let _lock = LOCK.write().unwrap();
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
