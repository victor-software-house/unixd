use std::{fmt, io};

/// Why a cache operation failed.
///
/// No variant carries a path, a key value, or file contents, so an error is
/// safe to log and to return across a process boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The root, or a directory or lock file under it, is a symlink, the wrong
    /// kind of file, or owned by another user.
    UnsafeRoot,
    /// A filesystem call failed.
    Io(io::ErrorKind),
    /// A file lock could not be taken.
    Lock,
    /// The same key part name was given twice.
    DuplicateKeyPart(String),
    /// A value could not be serialized.
    Encode,
}

impl Error {
    pub(crate) fn io(error: &io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafeRoot => formatter.write_str("cache root is not a private directory"),
            Self::Io(kind) => write!(formatter, "cache I/O failed: {kind}"),
            Self::Lock => formatter.write_str("cache lock could not be taken"),
            Self::DuplicateKeyPart(name) => write!(formatter, "key part `{name}` given twice"),
            Self::Encode => formatter.write_str("value could not be serialized"),
        }
    }
}

impl std::error::Error for Error {}
