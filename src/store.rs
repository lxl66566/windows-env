//! Registry backend abstraction for environment variable storage.
//!
//! [`EnvStore`] decouples the list-manipulation logic from the actual
//! storage, so tests can run against an in-memory implementation instead of
//! the real registry.

use std::{borrow::Cow, io, iter::once};

use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ},
    enums::RegType,
    types::FromRegValue,
    RegKey, RegValue,
};

use crate::{Error, Result};

/// The type of a registry string value.
///
/// Only the types meaningful for environment variables are represented;
/// reading any other type fails with [`Error::UnsupportedValueType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    /// `REG_SZ`
    Sz,
    /// `REG_EXPAND_SZ`; `%VAR%` placeholders are expanded by consumers.
    ExpandSz,
}

impl From<ValueKind> for RegType {
    fn from(kind: ValueKind) -> Self {
        match kind {
            ValueKind::Sz => REG_SZ,
            ValueKind::ExpandSz => REG_EXPAND_SZ,
        }
    }
}

impl TryFrom<RegType> for ValueKind {
    type Error = Error;

    fn try_from(vtype: RegType) -> Result<Self> {
        match vtype {
            REG_SZ => Ok(ValueKind::Sz),
            REG_EXPAND_SZ => Ok(ValueKind::ExpandSz),
            other => Err(Error::UnsupportedValueType(format!("{other:?}"))),
        }
    }
}

/// Storage backend for environment variables.
pub(crate) trait EnvStore {
    /// Read a value together with its type; `Ok(None)` if absent.
    fn get(&self, var: &str) -> Result<Option<(String, ValueKind)>>;

    /// Write a string value of the given type.
    fn set(&self, var: &str, value: &str, kind: ValueKind) -> Result<()>;

    /// Delete a value. Deleting an absent value is not an error.
    fn delete(&self, var: &str) -> Result<()>;
}

/// [`EnvStore`] backed by the real Windows registry (`HKCU\Environment`).
pub(crate) struct WinRegStore {
    key: RegKey,
}

impl WinRegStore {
    /// Open with read-only access.
    pub fn read_only() -> Result<Self> {
        Self::open(KEY_READ)
    }

    /// Open with read-write access.
    pub fn writable() -> Result<Self> {
        Self::open(KEY_READ | KEY_WRITE)
    }

    fn open(access: u32) -> Result<Self> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey_with_flags("Environment", access)?;
        Ok(Self { key })
    }
}

impl EnvStore for WinRegStore {
    fn get(&self, var: &str) -> Result<Option<(String, ValueKind)>> {
        match self.key.get_raw_value(var) {
            Ok(rv) => Ok(Some((
                String::from_reg_value(&rv)?,
                ValueKind::try_from(rv.vtype)?,
            ))),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn set(&self, var: &str, value: &str, kind: ValueKind) -> Result<()> {
        self.key.set_raw_value(var, &raw_string_value(value, kind))?;
        Ok(())
    }

    fn delete(&self, var: &str) -> Result<()> {
        match self.key.delete_value(var) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

/// Build a registry string value of the given kind from a UTF-8 string.
fn raw_string_value(s: &str, kind: ValueKind) -> RegValue<'static> {
    let bytes: Vec<u8> = s
        .encode_utf16()
        .chain(once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    RegValue {
        bytes: Cow::Owned(bytes),
        vtype: kind.into(),
    }
}
