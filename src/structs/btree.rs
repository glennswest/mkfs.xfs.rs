//! Short-form (per-AG) btree blocks: `struct xfs_btree_block` with the v5
//! `xfs_btree_block_shdr`, and the records of each per-AG btree.

use crate::bytes::*;
use crate::crc;
use crate::structs::NULL32;

/// Magic numbers of the v5 per-AG btrees.
#[allow(missing_docs)]
pub mod magic {
    /// By-block free space, "AB3B".
    pub const BNO: u32 = 0x4142_3342;
    /// By-size free space, "AB3C".
    pub const CNT: u32 = 0x4142_3343;
    /// Inodes, "IAB3".
    pub const INO: u32 = 0x4941_4233;
    /// Free inodes, "FIB3".
    pub const FINO: u32 = 0x4649_4233;
    /// Reverse mappings, "RMB3".
    pub const RMAP: u32 = 0x524d_4233;
    /// Reference counts, "R3FC".
    pub const REFC: u32 = 0x5233_4643;
}

/// Field offsets in a short-form v5 btree block header.
#[allow(missing_docs)]
pub mod off {
    pub const MAGIC: usize = 0;
    pub const LEVEL: usize = 4;
    pub const NUMRECS: usize = 6;
    pub const LEFTSIB: usize = 8;
    pub const RIGHTSIB: usize = 12;
    pub const BLKNO: usize = 16;
    pub const LSN: usize = 24;
    pub const UUID: usize = 32;
    pub const OWNER: usize = 48;
    pub const CRC: usize = 52;
    /// Records start here.
    pub const RECS: usize = 56;
}

/// Size of a free space record (`struct xfs_alloc_rec`).
pub const ALLOC_REC: usize = 8;
/// Size of an inode btree record (`struct xfs_inobt_rec`).
pub const INOBT_REC: usize = 16;
/// Size of a reverse mapping record (`struct xfs_rmap_rec`).
pub const RMAP_REC: usize = 24;
/// Size of a refcount record (`struct xfs_refcount_rec`).
pub const REFC_REC: usize = 12;

/// Special owners in the reverse mapping btree (`XFS_RMAP_OWN_*`).
#[allow(missing_docs)]
pub mod owner {
    pub const FS: u64 = -3i64 as u64;
    pub const LOG: u64 = -4i64 as u64;
    pub const AG: u64 = -5i64 as u64;
    pub const INOBT: u64 = -6i64 as u64;
    pub const INODES: u64 = -7i64 as u64;
    pub const REFC: u64 = -8i64 as u64;
}

/// A free extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllocRec {
    /// First block.
    pub start: u32,
    /// Length in blocks.
    pub count: u32,
}

/// An inode chunk (sparse record format when `sparse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InobtRec {
    /// First inode (AG-relative).
    pub startino: u32,
    /// Sparse chunks: which 4-inode groups are holes.
    pub holemask: u16,
    /// Inodes present.
    pub count: u8,
    /// Free inodes.
    pub freecount: u8,
    /// Free bitmap, one bit per inode.
    pub free: u64,
}

/// A reverse mapping of a special owner (no offset or flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RmapRec {
    /// First block.
    pub start: u32,
    /// Length in blocks.
    pub count: u32,
    /// `XFS_RMAP_OWN_*` or an inode number.
    pub owner: u64,
}

/// Build a single-level (leaf root) btree block of `blocksize` bytes at
/// disk address `daddr`, holding `recs` already encoded, and stamp its CRC.
/// What `xfs_btree_init_buf` and the records inserted after it produce.
pub fn leaf_block(
    magic: u32,
    daddr: u64,
    agno: u32,
    uuid: &[u8; 16],
    recs: &[u8],
    numrecs: u16,
    blocksize: usize,
) -> Vec<u8> {
    use off::*;
    let mut b = vec![0u8; blocksize];
    put32(&mut b, MAGIC, magic);
    put16(&mut b, LEVEL, 0);
    put16(&mut b, NUMRECS, numrecs);
    put32(&mut b, LEFTSIB, NULL32);
    put32(&mut b, RIGHTSIB, NULL32);
    put64(&mut b, BLKNO, daddr);
    put64(&mut b, LSN, 0);
    b[UUID..UUID + 16].copy_from_slice(uuid);
    put32(&mut b, OWNER, agno);
    b[RECS..RECS + recs.len()].copy_from_slice(recs);
    crc::stamp(&mut b, CRC);
    b
}

/// Encode free space records.
pub fn alloc_recs(recs: &[AllocRec]) -> Vec<u8> {
    let mut out = vec![0u8; recs.len() * ALLOC_REC];
    for (i, r) in recs.iter().enumerate() {
        put32(&mut out, i * ALLOC_REC, r.start);
        put32(&mut out, i * ALLOC_REC + 4, r.count);
    }
    out
}

/// Encode inode btree records. Without sparse inodes the free count is a
/// 32-bit field where the sparse format keeps holemask, count and freecount.
pub fn inobt_recs(recs: &[InobtRec], sparse: bool) -> Vec<u8> {
    let mut out = vec![0u8; recs.len() * INOBT_REC];
    for (i, r) in recs.iter().enumerate() {
        let o = i * INOBT_REC;
        put32(&mut out, o, r.startino);
        if sparse {
            put16(&mut out, o + 4, r.holemask);
            out[o + 6] = r.count;
            out[o + 7] = r.freecount;
        } else {
            put32(&mut out, o + 4, u32::from(r.freecount));
        }
        put64(&mut out, o + 8, r.free);
    }
    out
}

/// Encode reverse mapping records.
pub fn rmap_recs(recs: &[RmapRec]) -> Vec<u8> {
    let mut out = vec![0u8; recs.len() * RMAP_REC];
    for (i, r) in recs.iter().enumerate() {
        let o = i * RMAP_REC;
        put32(&mut out, o, r.start);
        put32(&mut out, o + 4, r.count);
        put64(&mut out, o + 8, r.owner);
        put64(&mut out, o + 16, 0);
    }
    out
}
