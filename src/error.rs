//! Errors surfaced by formatting and inspecting a filesystem.

use std::io;

/// Anything that can go wrong formatting or reading an XFS filesystem.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The underlying device failed a read, write or flush.
    #[error("device I/O failed at offset {offset}: {source}")]
    Io {
        /// Byte offset the operation targeted.
        offset: u64,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// A read or write ran past the end of the device.
    #[error("I/O of {len} bytes at offset {offset} runs past the end of the {size}-byte device")]
    OutOfBounds {
        /// Byte offset the operation targeted.
        offset: u64,
        /// Length requested.
        len: u64,
        /// Size of the device.
        size: u64,
    },

    /// The parameters cannot describe a filesystem `mkfs.xfs` would make.
    #[error("invalid parameters: {0}")]
    InvalidParams(String),

    /// A configuration `mkfs.xfs` accepts but this implementation does not
    /// yet reproduce exactly. Refused rather than approximated.
    #[error("not supported yet: {0}")]
    Unsupported(String),

    /// What was read is not an XFS filesystem, or not one this crate reads.
    #[error("not a readable XFS filesystem: {0}")]
    NotXfs(String),

    /// On-disk metadata failed a structural check.
    #[error("corrupt metadata: {0}")]
    Corrupt(String),
}

impl Error {
    /// Wrap an I/O failure with the offset it happened at.
    pub fn io(offset: u64, source: io::Error) -> Self {
        Error::Io { offset, source }
    }
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
