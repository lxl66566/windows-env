//! Error types of this crate.

use std::io;

use thiserror::Error;

/// Error type of this crate.
#[derive(Debug, Error)]
pub enum Error {
    /// The variable name is empty or contains `;` or NUL.
    #[error("invalid variable name {0:?}: must be non-empty and contain no `;` or NUL")]
    InvalidVarName(String),

    /// The list value is empty or contains `;` or NUL.
    #[error("invalid list value {0:?}: must be non-empty and contain no `;` or NUL")]
    InvalidListValue(String),

    /// The value contains a NUL character.
    #[error("invalid value: must not contain NUL")]
    InvalidValue,

    /// The registry value has a type other than `REG_SZ` / `REG_EXPAND_SZ`.
    #[error("unsupported registry value type: {0}")]
    UnsupportedValueType(String),

    /// An error from the underlying registry or Win32 call.
    #[error(transparent)]
    Registry(#[from] io::Error),
}

impl From<Error> for io::Error {
    /// Convert into [`io::Error`] so that callers using `io::Result` can keep
    /// using `?`. An inner [`io::Error`] is extracted as-is to avoid double
    /// wrapping; validation errors become [`io::ErrorKind::InvalidInput`].
    fn from(err: Error) -> Self {
        match err {
            Error::Registry(inner) => inner,
            other @ Error::UnsupportedValueType(_) => {
                io::Error::new(io::ErrorKind::InvalidData, other)
            }
            other => io::Error::new(io::ErrorKind::InvalidInput, other),
        }
    }
}

/// Result alias of this crate.
pub type Result<T> = std::result::Result<T, Error>;
