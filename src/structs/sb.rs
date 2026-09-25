//! The superblock: `struct xfs_dsb`.

use crate::bytes::*;
use crate::crc;
use crate::geometry::Geometry;

/// `XFS_SB_MAGIC`, "XFSB".
pub const MAGIC: u32 = 0x5846_5342;

/// Field offsets in `struct xfs_dsb`.
#[allow(missing_docs)]
pub mod off {
    pub const MAGICNUM: usize = 0;
    pub const BLOCKSIZE: usize = 4;
    pub const DBLOCKS: usize = 8;
    pub const RBLOCKS: usize = 16;
    pub const REXTENTS: usize = 24;
    pub const UUID: usize = 32;
    pub const LOGSTART: usize = 48;
    pub const ROOTINO: usize = 56;
    pub const RBMINO: usize = 64;
    pub const RSUMINO: usize = 72;
    pub const REXTSIZE: usize = 80;
    pub const AGBLOCKS: usize = 84;
    pub const AGCOUNT: usize = 88;
    pub const RBMBLOCKS: usize = 92;
    pub const LOGBLOCKS: usize = 96;
    pub const VERSIONNUM: usize = 100;
    pub const SECTSIZE: usize = 102;
    pub const INODESIZE: usize = 104;
    pub const INOPBLOCK: usize = 106;
    pub const FNAME: usize = 108;
    pub const BLOCKLOG: usize = 120;
    pub const SECTLOG: usize = 121;
    pub const INODELOG: usize = 122;
    pub const INOPBLOG: usize = 123;
    pub const AGBLKLOG: usize = 124;
    pub const REXTSLOG: usize = 125;
    pub const INPROGRESS: usize = 126;
    pub const IMAX_PCT: usize = 127;
    pub const ICOUNT: usize = 128;
    pub const IFREE: usize = 136;
    pub const FDBLOCKS: usize = 144;
    pub const FREXTENTS: usize = 152;
    pub const UQUOTINO: usize = 160;
    pub const GQUOTINO: usize = 168;
    pub const QFLAGS: usize = 176;
    pub const FLAGS: usize = 178;
    pub const SHARED_VN: usize = 179;
    pub const INOALIGNMT: usize = 180;
    pub const UNIT: usize = 184;
    pub const WIDTH: usize = 188;
    pub const DIRBLKLOG: usize = 192;
    pub const LOGSECTLOG: usize = 193;
    pub const LOGSECTSIZE: usize = 194;
    pub const LOGSUNIT: usize = 196;
    pub const FEATURES2: usize = 200;
    pub const BAD_FEATURES2: usize = 204;
    pub const FEATURES_COMPAT: usize = 208;
    pub const FEATURES_RO_COMPAT: usize = 212;
    pub const FEATURES_INCOMPAT: usize = 216;
    pub const FEATURES_LOG_INCOMPAT: usize = 220;
    pub const CRC: usize = 224;
    pub const SPINO_ALIGN: usize = 228;
    pub const PQUOTINO: usize = 232;
    pub const LSN: usize = 240;
    pub const META_UUID: usize = 248;
    /// End of the fields written for a filesystem without metadir.
    pub const END: usize = 264;
}

/// `sb_versionnum` bits.
#[allow(missing_docs)]
pub mod version {
    pub const V5: u16 = 5;
    pub const ATTRBIT: u16 = 0x10;
    pub const NLINKBIT: u16 = 0x20;
    pub const ALIGNBIT: u16 = 0x80;
    pub const DALIGNBIT: u16 = 0x100;
    pub const LOGV2BIT: u16 = 0x400;
    pub const SECTORBIT: u16 = 0x800;
    pub const EXTFLGBIT: u16 = 0x1000;
    pub const DIRV2BIT: u16 = 0x2000;
    pub const MOREBITSBIT: u16 = 0x8000;
    /// `sb_features2` bits.
    pub const F2_LAZYSBCOUNT: u32 = 0x2;
    pub const F2_ATTR2: u32 = 0x8;
    pub const F2_PROJID32: u32 = 0x80;
    pub const F2_CRC: u32 = 0x100;
    /// `sb_features_ro_compat` bits.
    pub const RO_FINOBT: u32 = 0x1;
    pub const RO_RMAPBT: u32 = 0x2;
    pub const RO_REFLINK: u32 = 0x4;
    pub const RO_INOBTCNT: u32 = 0x8;
    /// `sb_features_incompat` bits.
    pub const IN_FTYPE: u32 = 0x1;
    pub const IN_SPINODES: u32 = 0x2;
    pub const IN_META_UUID: u32 = 0x4;
    pub const IN_BIGTIME: u32 = 0x8;
    pub const IN_NEEDSREPAIR: u32 = 0x10;
    pub const IN_NREXT64: u32 = 0x20;
}

/// The fields that differ between the primary superblock and the
/// secondaries, and between the start and end of a format.
#[derive(Debug, Clone, Copy)]
pub struct SbState {
    /// Filesystem UUID.
    pub uuid: [u8; 16],
    /// Root directory inode, or `NULLFSINO`.
    pub rootino: u64,
    /// Realtime bitmap inode, or `NULLFSINO`.
    pub rbmino: u64,
    /// Realtime summary inode, or `NULLFSINO`.
    pub rsumino: u64,
    /// Allocated inodes.
    pub icount: u64,
    /// Free inodes.
    pub ifree: u64,
    /// Free data blocks.
    pub fdblocks: u64,
    /// `mkfs` has not finished.
    pub inprogress: bool,
}

/// `sb_versionnum`, as `sb_set_features` builds it.
pub fn versionnum(g: &Geometry) -> u16 {
    let mut v = version::V5 | version::NLINKBIT | version::EXTFLGBIT | version::DIRV2BIT;
    v |= version::ALIGNBIT | version::LOGV2BIT;
    if g.sectsize > 512 || g.lsectsize > 512 {
        v |= version::SECTORBIT;
    }
    // features2 is never empty on a v5 filesystem.
    v | version::MOREBITSBIT
}

/// `sb_features2` (and `sb_bad_features2`).
pub fn features2(_g: &Geometry) -> u32 {
    version::F2_LAZYSBCOUNT | version::F2_PROJID32 | version::F2_CRC | version::F2_ATTR2
}

/// `sb_features_ro_compat`.
pub fn ro_compat(g: &Geometry) -> u32 {
    let f = g.features;
    let mut r = 0;
    if f.finobt {
        r |= version::RO_FINOBT;
    }
    if f.rmapbt {
        r |= version::RO_RMAPBT;
    }
    if f.reflink {
        r |= version::RO_REFLINK;
    }
    if f.inobtcount {
        r |= version::RO_INOBTCNT;
    }
    r
}

/// `sb_features_incompat`.
pub fn incompat(g: &Geometry) -> u32 {
    let f = g.features;
    let mut r = version::IN_FTYPE;
    if f.bigtime {
        r |= version::IN_BIGTIME;
    }
    if f.sparse {
        r |= version::IN_SPINODES;
    }
    if f.nrext64 {
        r |= version::IN_NREXT64;
    }
    r
}

/// Encode a superblock into `sector` (one sector, zeroed by the caller)
/// and stamp its CRC, as `xfs_sb_to_disk` and the write verifier do.
pub fn encode(g: &Geometry, s: &SbState, sector: &mut [u8]) {
    use off::*;
    let b = sector;
    put32(b, MAGICNUM, MAGIC);
    put32(b, BLOCKSIZE, g.blocksize);
    put64(b, DBLOCKS, g.dblocks);
    put64(b, RBLOCKS, 0);
    put64(b, REXTENTS, 0);
    b[UUID..UUID + 16].copy_from_slice(&s.uuid);
    put64(b, LOGSTART, g.logstart());
    put64(b, ROOTINO, s.rootino);
    put64(b, RBMINO, s.rbmino);
    put64(b, RSUMINO, s.rsumino);
    put32(b, REXTSIZE, 1);
    put32(b, AGBLOCKS, g.agsize as u32);
    put32(b, AGCOUNT, g.agcount as u32);
    put32(b, RBMBLOCKS, 0);
    put32(b, LOGBLOCKS, g.logblocks as u32);
    put16(b, VERSIONNUM, versionnum(g));
    put16(b, SECTSIZE, g.sectsize as u16);
    put16(b, INODESIZE, g.inodesize as u16);
    put16(b, INOPBLOCK, g.inopblock as u16);
    b[FNAME..FNAME + 12].copy_from_slice(&g.label);
    b[BLOCKLOG] = g.blocklog;
    b[SECTLOG] = g.sectlog;
    b[INODELOG] = g.inodelog;
    b[INOPBLOG] = g.inopblog;
    b[AGBLKLOG] = g.agblklog;
    b[REXTSLOG] = 0;
    b[INPROGRESS] = u8::from(s.inprogress);
    b[IMAX_PCT] = g.imaxpct;
    put64(b, ICOUNT, s.icount);
    put64(b, IFREE, s.ifree);
    put64(b, FDBLOCKS, s.fdblocks);
    put64(b, FREXTENTS, 0);
    put64(b, UQUOTINO, 0);
    put64(b, GQUOTINO, 0);
    put32(b, INOALIGNMT, g.inoalignmt() as u32);
    put32(b, UNIT, 0);
    put32(b, WIDTH, 0);
    b[DIRBLKLOG] = g.dirblocklog - g.blocklog;
    if g.sectsize > 512 || g.lsectsize > 512 {
        b[LOGSECTLOG] = g.lsectlog;
        put16(b, LOGSECTSIZE, g.lsectsize as u16);
    }
    // A v2 log with no stripe unit records 1, never 0.
    put32(b, LOGSUNIT, 1);
    put32(b, FEATURES2, features2(g));
    put32(b, BAD_FEATURES2, features2(g));
    put32(b, FEATURES_COMPAT, 0);
    put32(b, FEATURES_RO_COMPAT, ro_compat(g));
    put32(b, FEATURES_INCOMPAT, incompat(g));
    put32(b, FEATURES_LOG_INCOMPAT, 0);
    put32(b, SPINO_ALIGN, g.spino_align());
    put64(b, PQUOTINO, 0);
    put64(b, LSN, 0);
    // sb_meta_uuid is written only with the META_UUID feature.
    crc::stamp(&mut b[..g.sectsize as usize], CRC);
}
