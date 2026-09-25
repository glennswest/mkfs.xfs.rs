//! Async **XFS** formatter in pure Rust — a from-scratch reimplementation of
//! `mkfs.xfs`, written from the XFS on-disk format and held to real
//! `mkfs.xfs` output: [`compare`] diffs every metadata field of two
//! filesystems, and the golden tests fail on any structural difference.
//!
//! ```no_run
//! # async fn f() -> mkfs_xfs::Result<()> {
//! use mkfs_xfs::{device::FileDevice, format::format, geometry::Params};
//! let dev = FileDevice::create("fs.img", 1 << 30).await?;
//! let report = format(&dev, &Params::new()).await?;
//! println!("{} AGs of {} blocks", report.geometry.agcount, report.geometry.agsize);
//! # Ok(()) }
//! ```
//!
//! | Module | What it owns |
//! |---|---|
//! | [`device`] | the async [`device::BlockDevice`] trait, file and sparse memory devices |
//! | [`geometry`] | what `mkfs.xfs` chooses: sizes, AG geometry, log size and place |
//! | [`structs`] | byte-exact on-disk structures |
//! | [`format`] | the formatter |
//! | [`inspect`] | read any v5 XFS filesystem's metadata into named fields |
//! | [`compare`] | diff two filesystems field by field |
//! | [`crc`] | CRC32C as XFS uses it |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod bytes;
pub mod compare;
pub mod crc;
pub mod device;
pub mod error;
pub mod format;
pub mod geometry;
pub mod inspect;
pub mod structs;

pub use error::{Error, Result};
