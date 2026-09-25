//! On-disk inodes: `struct xfs_dinode` (v3), followed by the data fork.

use crate::bytes::*;
use crate::crc;
use crate::geometry::Timestamp;
use crate::structs::NULL32;

/// `XFS_DINODE_MAGIC`, "IN".
pub const MAGIC: u16 = 0x494e;
/// Size of the v3 inode core; the data fork starts here.
pub const CORE_SIZE: usize = 176;

/// Field offsets in `struct xfs_dinode`.
#[allow(missing_docs)]
pub mod off {
    pub const MAGIC: usize = 0;
    pub const MODE: usize = 2;
    pub const VERSION: usize = 4;
    pub const FORMAT: usize = 5;
    pub const METATYPE: usize = 6;
    pub const UID: usize = 8;
    pub const GID: usize = 12;
    pub const NLINK: usize = 16;
    pub const PROJID_LO: usize = 20;
    pub const PROJID_HI: usize = 22;
    /// `di_big_nextents` with nrext64, padding otherwise.
    pub const BIG_NEXTENTS: usize = 24;
    pub const ATIME: usize = 32;
    pub const MTIME: usize = 40;
    pub const CTIME: usize = 48;
    pub const SIZE: usize = 56;
    pub const NBLOCKS: usize = 64;
    pub const EXTSIZE: usize = 72;
    /// `di_nextents` (u32), or `di_big_anextents` with nrext64.
    pub const NEXTENTS: usize = 76;
    /// `di_anextents` (u16), or padding with nrext64.
    pub const ANEXTENTS: usize = 80;
    pub const FORKOFF: usize = 82;
    pub const AFORMAT: usize = 83;
    pub const DMEVMASK: usize = 84;
    pub const DMSTATE: usize = 88;
    pub const FLAGS: usize = 90;
    pub const GEN: usize = 92;
    pub const NEXT_UNLINKED: usize = 96;
    pub const CRC: usize = 100;
    pub const CHANGECOUNT: usize = 104;
    pub const LSN: usize = 112;
    pub const FLAGS2: usize = 120;
    pub const COWEXTSIZE: usize = 128;
    pub const CRTIME: usize = 144;
    pub const INO: usize = 152;
    pub const UUID: usize = 160;
}

/// `XFS_DINODE_FMT_*`.
#[allow(missing_docs)]
pub mod format {
    pub const DEV: u8 = 0;
    pub const LOCAL: u8 = 1;
    pub const EXTENTS: u8 = 2;
    pub const BTREE: u8 = 3;
}

/// `XFS_DIFLAG_NEWRTBM`: the realtime bitmap inode's atime is a counter.
pub const DIFLAG_NEWRTBM: u16 = 1 << 2;
/// `XFS_DIFLAG2_BIGTIME`.
pub const DIFLAG2_BIGTIME: u64 = 1 << 3;
/// `XFS_DIFLAG2_NREXT64`.
pub const DIFLAG2_NREXT64: u64 = 1 << 4;
/// `XFS_BIGTIME_EPOCH_OFFSET`: bigtime counts from the minimum 32-bit time.
pub const BIGTIME_EPOCH_OFFSET: i64 = 1 << 31;

/// Encode a timestamp as the inode stores it.
pub fn encode_time(t: Timestamp, bigtime: bool) -> u64 {
    if bigtime {
        ((t.secs + BIGTIME_EPOCH_OFFSET) as u64) * 1_000_000_000 + u64::from(t.nsecs)
    } else {
        ((t.secs as u32 as u64) << 32) | u64::from(t.nsecs)
    }
}

/// Decode a stored timestamp.
pub fn decode_time(raw: u64, bigtime: bool) -> Timestamp {
    if bigtime {
        Timestamp {
            secs: (raw / 1_000_000_000) as i64 - BIGTIME_EPOCH_OFFSET,
            nsecs: (raw % 1_000_000_000) as u32,
        }
    } else {
        Timestamp { secs: (raw >> 32) as u32 as i32 as i64, nsecs: raw as u32 }
    }
}

/// An allocated inode, as `mkfs.xfs` creates them (no attribute fork).
#[derive(Debug, Clone)]
pub struct Dinode {
    /// Inode number.
    pub ino: u64,
    /// Type and permissions.
    pub mode: u16,
    /// Data fork format.
    pub format: u8,
    /// Link count.
    pub nlink: u32,
    /// File size in bytes.
    pub size: u64,
    /// `di_flags`.
    pub flags: u16,
    /// `di_flags2`.
    pub flags2: u64,
    /// Generation number.
    pub gen: u32,
    /// `di_changecount`.
    pub changecount: u64,
    #[allow(missing_docs)]
    pub atime: Timestamp,
    #[allow(missing_docs)]
    pub mtime: Timestamp,
    #[allow(missing_docs)]
    pub ctime: Timestamp,
    #[allow(missing_docs)]
    pub crtime: Timestamp,
    /// The data fork's bytes (a short-form directory, say).
    pub data: Vec<u8>,
}

impl Dinode {
    /// Encode into `slot` (one inode, zeroed by the caller) and stamp the CRC.
    pub fn encode(&self, uuid: &[u8; 16], slot: &mut [u8]) {
        use off::*;
        let b = slot;
        let bigtime = self.flags2 & DIFLAG2_BIGTIME != 0;
        put16(b, MAGIC, super::inode::MAGIC);
        put16(b, MODE, self.mode);
        b[VERSION] = 3;
        b[FORMAT] = self.format;
        put32(b, NLINK, self.nlink);
        put64(b, ATIME, encode_time(self.atime, bigtime));
        put64(b, MTIME, encode_time(self.mtime, bigtime));
        put64(b, CTIME, encode_time(self.ctime, bigtime));
        put64(b, SIZE, self.size);
        b[FORKOFF] = 0;
        b[AFORMAT] = format::EXTENTS;
        put16(b, FLAGS, self.flags);
        put32(b, GEN, self.gen);
        put32(b, NEXT_UNLINKED, NULL32);
        put64(b, CHANGECOUNT, self.changecount);
        put64(b, FLAGS2, self.flags2);
        put64(b, CRTIME, encode_time(self.crtime, bigtime));
        put64(b, INO, self.ino);
        b[UUID..UUID + 16].copy_from_slice(uuid);
        b[CORE_SIZE..CORE_SIZE + self.data.len()].copy_from_slice(&self.data);
        crc::stamp(b, CRC);
    }
}

/// Encode a never-allocated inode into `slot`, as `xfs_ialloc_inode_init`
/// writes every inode of a new chunk: magic, version, generation, an empty
/// unlinked pointer, its number and the UUID.
pub fn encode_free(ino: u64, gen: u32, uuid: &[u8; 16], slot: &mut [u8]) {
    use off::*;
    put16(slot, MAGIC, super::inode::MAGIC);
    slot[VERSION] = 3;
    put32(slot, GEN, gen);
    put32(slot, NEXT_UNLINKED, NULL32);
    put64(slot, INO, ino);
    slot[UUID..UUID + 16].copy_from_slice(uuid);
    crc::stamp(slot, CRC);
}

/// The data fork of an empty short-form directory (`xfs_dir2_sf_hdr`):
/// no entries, 4-byte inode numbers, `parent` as its parent.
pub fn empty_sf_dir(parent: u64) -> Vec<u8> {
    if parent > u64::from(u32::MAX) {
        // xfs_dir2_sf_create: a parent past 32 bits makes i8count 1.
        let mut d = vec![0u8; 10];
        d[1] = 1;
        d[2..10].copy_from_slice(&parent.to_be_bytes());
        d
    } else {
        let mut d = vec![0u8; 6];
        d[2..6].copy_from_slice(&(parent as u32).to_be_bytes());
        d
    }
}
