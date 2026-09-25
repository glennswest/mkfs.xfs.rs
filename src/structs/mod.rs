//! Byte-exact on-disk structures, encoded field by field in big-endian.
//!
//! Every structure names the `libxfs/xfs_format.h` (or `xfs_log_format.h`)
//! struct it mirrors, and every field offset is a named constant used by
//! both the encoder here and the reader in [`crate::inspect`].

pub mod ag;
pub mod btree;
pub mod inode;
pub mod log;
pub mod sb;

/// `NULLAGBLOCK` / `NULLAGINO`.
pub const NULL32: u32 = 0xffff_ffff;
/// `NULLFSINO`.
pub const NULL64: u64 = u64::MAX;
