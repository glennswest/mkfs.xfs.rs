//! Allocation group headers: `struct xfs_agf`, `struct xfs_agi` and
//! `struct xfs_agfl`, one sector each after the superblock.

use crate::bytes::*;
use crate::crc;
use crate::structs::NULL32;

/// `XFS_AGF_MAGIC`, "XAGF".
pub const AGF_MAGIC: u32 = 0x5841_4746;
/// `XFS_AGI_MAGIC`, "XAGI".
pub const AGI_MAGIC: u32 = 0x5841_4749;
/// `XFS_AGFL_MAGIC`, "XAFL".
pub const AGFL_MAGIC: u32 = 0x5841_464c;
/// `XFS_AGF_VERSION` / `XFS_AGI_VERSION`.
pub const AG_VERSION: u32 = 1;
/// `XFS_AGI_UNLINKED_BUCKETS`.
pub const UNLINKED_BUCKETS: usize = 64;

/// Field offsets in `struct xfs_agf`.
#[allow(missing_docs)]
pub mod agf {
    pub const MAGICNUM: usize = 0;
    pub const VERSIONNUM: usize = 4;
    pub const SEQNO: usize = 8;
    pub const LENGTH: usize = 12;
    pub const BNO_ROOT: usize = 16;
    pub const CNT_ROOT: usize = 20;
    pub const RMAP_ROOT: usize = 24;
    pub const BNO_LEVEL: usize = 28;
    pub const CNT_LEVEL: usize = 32;
    pub const RMAP_LEVEL: usize = 36;
    pub const FLFIRST: usize = 40;
    pub const FLLAST: usize = 44;
    pub const FLCOUNT: usize = 48;
    pub const FREEBLKS: usize = 52;
    pub const LONGEST: usize = 56;
    pub const BTREEBLKS: usize = 60;
    pub const UUID: usize = 64;
    pub const RMAP_BLOCKS: usize = 80;
    pub const REFCOUNT_BLOCKS: usize = 84;
    pub const REFCOUNT_ROOT: usize = 88;
    pub const REFCOUNT_LEVEL: usize = 92;
    pub const LSN: usize = 208;
    pub const CRC: usize = 216;
    pub const END: usize = 224;
}

/// Field offsets in `struct xfs_agi`.
#[allow(missing_docs)]
pub mod agi {
    pub const MAGICNUM: usize = 0;
    pub const VERSIONNUM: usize = 4;
    pub const SEQNO: usize = 8;
    pub const LENGTH: usize = 12;
    pub const COUNT: usize = 16;
    pub const ROOT: usize = 20;
    pub const LEVEL: usize = 24;
    pub const FREECOUNT: usize = 28;
    pub const NEWINO: usize = 32;
    pub const DIRINO: usize = 36;
    pub const UNLINKED: usize = 40;
    pub const UUID: usize = 296;
    pub const CRC: usize = 312;
    pub const LSN: usize = 320;
    pub const FREE_ROOT: usize = 328;
    pub const FREE_LEVEL: usize = 332;
    pub const IBLOCKS: usize = 336;
    pub const FBLOCKS: usize = 340;
    pub const END: usize = 344;
}

/// Field offsets in `struct xfs_agfl`; the free list follows the header.
#[allow(missing_docs)]
pub mod agfl {
    pub const MAGICNUM: usize = 0;
    pub const SEQNO: usize = 4;
    pub const UUID: usize = 8;
    pub const LSN: usize = 24;
    pub const CRC: usize = 32;
    pub const BNO: usize = 36;
}

/// Everything in an AGF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agf {
    /// AG number.
    pub seqno: u32,
    /// Blocks in this AG.
    pub length: u32,
    /// By-block free space btree root and level.
    pub bno_root: u32,
    #[allow(missing_docs)]
    pub bno_level: u32,
    /// By-size free space btree root and level.
    pub cnt_root: u32,
    #[allow(missing_docs)]
    pub cnt_level: u32,
    /// Reverse mapping btree root, level and block count (0 without rmapbt).
    pub rmap_root: u32,
    #[allow(missing_docs)]
    pub rmap_level: u32,
    #[allow(missing_docs)]
    pub rmap_blocks: u32,
    /// Refcount btree root, level and block count (0 without reflink).
    pub refcount_root: u32,
    #[allow(missing_docs)]
    pub refcount_level: u32,
    #[allow(missing_docs)]
    pub refcount_blocks: u32,
    /// Free list: first and last slot used, and how many.
    pub flfirst: u32,
    #[allow(missing_docs)]
    pub fllast: u32,
    #[allow(missing_docs)]
    pub flcount: u32,
    /// Free blocks in the free space btrees.
    pub freeblks: u32,
    /// Longest free extent.
    pub longest: u32,
    /// Free space btree blocks beyond the roots.
    pub btreeblks: u32,
}

impl Agf {
    /// Encode into `sector` (zeroed by the caller) and stamp the CRC.
    pub fn encode(&self, uuid: &[u8; 16], sector: &mut [u8]) {
        use agf::*;
        let b = sector;
        put32(b, MAGICNUM, AGF_MAGIC);
        put32(b, VERSIONNUM, AG_VERSION);
        put32(b, SEQNO, self.seqno);
        put32(b, LENGTH, self.length);
        put32(b, BNO_ROOT, self.bno_root);
        put32(b, CNT_ROOT, self.cnt_root);
        put32(b, RMAP_ROOT, self.rmap_root);
        put32(b, BNO_LEVEL, self.bno_level);
        put32(b, CNT_LEVEL, self.cnt_level);
        put32(b, RMAP_LEVEL, self.rmap_level);
        put32(b, FLFIRST, self.flfirst);
        put32(b, FLLAST, self.fllast);
        put32(b, FLCOUNT, self.flcount);
        put32(b, FREEBLKS, self.freeblks);
        put32(b, LONGEST, self.longest);
        put32(b, BTREEBLKS, self.btreeblks);
        b[UUID..UUID + 16].copy_from_slice(uuid);
        put32(b, RMAP_BLOCKS, self.rmap_blocks);
        put32(b, REFCOUNT_BLOCKS, self.refcount_blocks);
        put32(b, REFCOUNT_ROOT, self.refcount_root);
        put32(b, REFCOUNT_LEVEL, self.refcount_level);
        put64(b, LSN, 0);
        let len = b.len();
        crc::stamp(&mut b[..len], CRC);
    }
}

/// Everything in an AGI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agi {
    /// AG number.
    pub seqno: u32,
    /// Blocks in this AG.
    pub length: u32,
    /// Allocated inodes.
    pub count: u32,
    /// Inode btree root and level.
    pub root: u32,
    #[allow(missing_docs)]
    pub level: u32,
    /// Free inodes.
    pub freecount: u32,
    /// The most recently allocated chunk's first inode, or `NULLAGINO`.
    pub newino: u32,
    /// Free inode btree root and level (0 without finobt).
    pub free_root: u32,
    #[allow(missing_docs)]
    pub free_level: u32,
    /// Inode and free inode btree block counts (0 without inobtcount).
    pub iblocks: u32,
    #[allow(missing_docs)]
    pub fblocks: u32,
}

impl Agi {
    /// Encode into `sector` (zeroed by the caller) and stamp the CRC.
    pub fn encode(&self, uuid: &[u8; 16], sector: &mut [u8]) {
        use agi::*;
        let b = sector;
        put32(b, MAGICNUM, AGI_MAGIC);
        put32(b, VERSIONNUM, AG_VERSION);
        put32(b, SEQNO, self.seqno);
        put32(b, LENGTH, self.length);
        put32(b, COUNT, self.count);
        put32(b, ROOT, self.root);
        put32(b, LEVEL, self.level);
        put32(b, FREECOUNT, self.freecount);
        put32(b, NEWINO, self.newino);
        put32(b, DIRINO, NULL32);
        for i in 0..UNLINKED_BUCKETS {
            put32(b, UNLINKED + 4 * i, NULL32);
        }
        b[UUID..UUID + 16].copy_from_slice(uuid);
        put64(b, LSN, 0);
        put32(b, FREE_ROOT, self.free_root);
        put32(b, FREE_LEVEL, self.free_level);
        put32(b, IBLOCKS, self.iblocks);
        put32(b, FBLOCKS, self.fblocks);
        let len = b.len();
        crc::stamp(&mut b[..len], CRC);
    }
}

/// Encode an AGFL into `sector`: the header, then `list` in slots
/// `first..`, every other slot `NULLAGBLOCK`.
pub fn encode_agfl(seqno: u32, uuid: &[u8; 16], first: u32, list: &[u32], sector: &mut [u8]) {
    use agfl::*;
    let b = sector;
    put32(b, MAGICNUM, AGFL_MAGIC);
    put32(b, SEQNO, seqno);
    b[UUID..UUID + 16].copy_from_slice(uuid);
    put64(b, LSN, 0);
    let slots = (b.len() - BNO) / 4;
    for i in 0..slots {
        put32(b, BNO + 4 * i, NULL32);
    }
    for (i, &bno) in list.iter().enumerate() {
        let slot = (first as usize + i) % slots;
        put32(b, BNO + 4 * slot, bno);
    }
    let len = b.len();
    crc::stamp(&mut b[..len], CRC);
}
