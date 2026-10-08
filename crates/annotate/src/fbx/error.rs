//! Why an FBX could not be read or patched.

use thiserror::Error;

/// Why an FBX could not be read or patched. The text is for logs and the CLI;
/// the viewer shows its own localized notice and carries this as the detail.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum FbxError {
    #[error("not a binary FBX")]
    NotBinary,
    #[error("not an FBX file")]
    NotFbx,
    #[error("FBX version {0} is not supported (FBX 7.0 or newer is required)")]
    UnsupportedVersion(u32),
    #[error("the file ends early, at byte {offset}")]
    Truncated { offset: usize },
    #[error("the file is malformed at byte {offset}")]
    Malformed { offset: usize },
    #[error("a property list could not be read")]
    BadProperty,
    #[error("the edited file would exceed what this FBX version can hold")]
    TooLarge,
    #[error("the file has no Objects section")]
    NoObjects,
    #[error("no object with id {0} in the file")]
    NoSuchObject(i64),
    #[error("ASCII FBX syntax error on line {line}")]
    AsciiSyntax { line: usize },
    #[error("ASCII FBX 6.x files are not supported")]
    AsciiLegacy,
}

pub type FbxResult<T> = Result<T, FbxError>;
