//! This crate provides a wrapper for Windows variable operations.

use std::{io, sync::RwLock};

use windows::{
    core::{HSTRING, PCWSTR},
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

/// Get the current Windows environment variable RegKey.
fn regkey() -> io::Result<RegKey> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
}

/// Append a value at the end to the Windows environment variable list
/// (separated by `;`).
///
/// If the value already exists, it will not be added again.
pub fn append<T1, T2>(var: T1, value: T2) -> io::Result<()>
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
pub fn prepend<T1, T2>(var: T1, value: T2) -> io::Result<()>
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    add_inner(var.as_ref(), value.as_ref(), true)
}

fn add_inner(var: &str, value: &str, front: bool) -> io::Result<()> {
    let _lock = LOCK.write().unwrap();
    let env = regkey()?;
    let get_res = env.get_value(var);
    let env_var: String = match get_res {
        Ok(s) => s,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::default(),
        Err(err) => return Err(err),
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
        env.set_value(var, &values.join(";"))?;
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
pub fn remove_from_list(var: &str, value: &str) -> io::Result<bool> {
    let _lock = LOCK.write().unwrap();
    let env = regkey()?;
    let get_res = env.get_value(var);
    let env_var: String = match get_res {
        Ok(s) => s,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    let mut values = env_var.split(';').collect::<Vec<&str>>();
    let len = values.len();
    values.retain(|p| p != &value);
    let found = len != values.len();
    env.set_value(var, &values.join(";"))?;
    notify_system();
    Ok(found)
}

/// Check if a value exists in the Windows environment variable list (separated
/// by `;`).
pub fn exists_in_list(var: &str, value: &str) -> io::Result<bool> {
    // locked in `get`
    let env_var = get(var)?;
    match env_var {
        Some(s) => Ok(s.split(';').any(|p| p == value)),
        None => Ok(false),
    }
}

/// Set a var in the Windows environment variable.
pub fn set<T1: AsRef<str>, T2: AsRef<str>>(var: T1, value: T2) -> io::Result<()> {
    let _lock = LOCK.write().unwrap();
    let env = regkey()?;
    env.set_value(var.as_ref(), &value.as_ref())?;
    notify_system();
    Ok(())
}

/// Get a var from the Windows environment variable.
pub fn get<T: AsRef<str>>(var: T) -> io::Result<Option<String>> {
    let _lock = LOCK.read().unwrap();
    let env = regkey()?;
    let res = env.get_value(var.as_ref());
    match res {
        Ok(s) => Ok(Some(s)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Remove a var from the Windows environment variable.
pub fn remove<T: AsRef<str>>(var: T) -> io::Result<()> {
    let _lock = LOCK.write().unwrap();
    let env = regkey()?;
    if let Err(err) = env.delete_value(var.as_ref()) {
        if err.kind() != io::ErrorKind::NotFound {
            return Err(err);
        }
    };
    notify_system();
    Ok(())
}

/// Convert UTF-8 str to PCWSTR
fn w<T: Into<HSTRING>>(x: T) -> PCWSTR {
    PCWSTR::from_raw(x.into().as_ptr())
}

fn notify_system() {
    let msg = w("Environment");
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

    const ENV_VAR: &str = "WINDOWS-ENV-TEST";

    #[test]
    fn test_get_set() -> Result<(), Box<dyn std::error::Error>> {
        set(ENV_VAR, "test")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "test");
        remove(ENV_VAR)?;
        assert!(get(ENV_VAR)?.is_none());
        Ok(())
    }

    #[test]
    fn test_list_operations() -> Result<(), Box<dyn std::error::Error>> {
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
    fn test_reset_one_var() -> Result<(), Box<dyn std::error::Error>> {
        set(ENV_VAR, "test")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "test");
        set(ENV_VAR, "new_test")?;
        assert_eq!(get(ENV_VAR)?.unwrap(), "new_test");
        remove(ENV_VAR)?;
        Ok(())
    }

    #[test]
    fn test_operate_with_not_exist_var() -> Result<(), Box<dyn std::error::Error>> {
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
}
