//! The internal log as `mkfs.xfs` leaves it: zeroed, with one record at
//! its start holding an unmount record, so the kernel finds a clean log at
//! cycle 1 (`libxfs_log_clear` with `XLOG_INIT_CYCLE`, `libxfs_log_header`).
//! [`record`] is the general `libxfs_log_header`, which `xfs_admin -U`'s
//! log clear also writes at later cycles ([`crate::admin`]).

use crate::bytes::*;

/// `XLOG_HEADER_MAGIC_NUM`.
pub const HEADER_MAGIC: u32 = 0xfeed_babe;
/// `XLOG_BIG_RECORD_BSIZE`.
pub const BIG_RECORD_BSIZE: u32 = 32 * 1024;
/// `XLOG_HEADER_CYCLE_SIZE`.
pub const HEADER_CYCLE_SIZE: u32 = 32 * 1024;
/// `XLOG_FMT_LINUX_LE`: the log format of a little-endian host, which is
/// what `mkfs.xfs` on x86 and arm64 records.
pub const FMT_LINUX_LE: u32 = 1;
/// `XLOG_UNMOUNT_TYPE`.
pub const UNMOUNT_TYPE: u16 = 0x556e;
/// `XFS_TRANSACTION`... the unmount record's ticket id (`oh_tid`).
pub const UNMOUNT_TID: u32 = 0xb0c0_d0d0;
/// `XFS_LOG` client id.
pub const CLIENT_LOG: u8 = 0xaa;
/// `XLOG_UNMOUNT_TRANS`.
pub const UNMOUNT_TRANS: u8 = 0x20;
/// Basic block size.
pub const BBSIZE: usize = 512;

/// Field offsets in `struct xlog_rec_header`.
#[allow(missing_docs)]
pub mod off {
    pub const MAGICNO: usize = 0;
    pub const CYCLE: usize = 4;
    pub const VERSION: usize = 8;
    pub const LEN: usize = 12;
    pub const LSN: usize = 16;
    pub const TAIL_LSN: usize = 24;
    pub const CRC: usize = 32;
    pub const PREV_BLOCK: usize = 36;
    pub const NUM_LOGOPS: usize = 40;
    pub const CYCLE_DATA: usize = 44;
    pub const FMT: usize = 300;
    pub const FS_UUID: usize = 304;
    pub const SIZE: usize = 320;
}

/// The stripe unit `libxfs_log_clear` is given (`prepare_devices`):
/// `sb_logsunit`, except that 1 — no stripe unit — stands for
/// `sb_logsectsize`, which is 0 for a 512-byte log sector.
pub fn clear_sunit(logsunit: u32, logsectsize: u32) -> u32 {
    if logsunit == 1 {
        logsectsize
    } else {
        logsunit
    }
}

/// `xlog_assign_lsn`: a log sequence number, cycle and basic block.
pub fn lsn(cycle: u32, block: u32) -> u64 {
    (u64::from(cycle) << 32) | u64::from(block)
}

/// The first log record: header, unmount record, and any remaining basic
/// blocks of the record stamped with the cycle. `sunit` in bytes, as
/// [`clear_sunit`] gives it.
pub fn first_record(uuid: &[u8; 16], sunit: u32) -> Vec<u8> {
    let l = lsn(1, 0);
    record(uuid, sunit, l, l)
}

/// One log record holding only an unmount record, as `libxfs_log_header`
/// writes it: `max(BTOBB(sunit), headers + 1)` basic blocks, at least two,
/// every block after the unmount record zero but for the cycle stamp.
/// `sunit` in bytes (0 for none).
pub fn record(uuid: &[u8; 16], sunit: u32, lsn: u64, tail_lsn: u64) -> Vec<u8> {
    let cycle = (lsn >> 32) as u32;
    let hdrs = if sunit > HEADER_CYCLE_SIZE { sunit.div_ceil(HEADER_CYCLE_SIZE) } else { 1 };
    let len_bb = if sunit > 0 { (sunit as usize).div_ceil(BBSIZE) } else { 1 }.max(hdrs as usize + 1);
    let mut rec = vec![0u8; len_bb * BBSIZE];
    {
        let h = &mut rec[..BBSIZE];
        put32(h, off::MAGICNO, HEADER_MAGIC);
        put32(h, off::CYCLE, cycle);
        put32(h, off::VERSION, 2);
        put32(h, off::CRC, 0);
        put32(h, off::PREV_BLOCK, u32::MAX);
        put32(h, off::NUM_LOGOPS, 1);
        put32(h, off::FMT, FMT_LINUX_LE);
        put32(h, off::SIZE, sunit.max(BIG_RECORD_BSIZE));
        put64(h, off::LSN, lsn);
        put64(h, off::TAIL_LSN, tail_lsn);
        h[off::FS_UUID..off::FS_UUID + 16].copy_from_slice(uuid);
        put32(h, off::LEN, (BBSIZE as u32 * 2).max(sunit) - hdrs * BBSIZE as u32);
    }
    // Extended headers, when the record needs more than one.
    for i in 1..hdrs as usize {
        put32(&mut rec[i * BBSIZE..], 0, cycle);
    }
    // The unmount record: an op header then struct xfs_unmount_log_format,
    // whose magic is host-endian.
    let u = hdrs as usize * BBSIZE;
    put32(&mut rec, u, UNMOUNT_TID);
    put32(&mut rec, u + 4, 8);
    rec[u + 8] = CLIENT_LOG;
    rec[u + 9] = UNMOUNT_TRANS;
    rec[u + 12..u + 14].copy_from_slice(&UNMOUNT_TYPE.to_le_bytes());
    // Its first word moves into the header's cycle data and the block is
    // stamped with the cycle, as every log block is.
    let first = be32(&rec, u);
    put32(&mut rec, off::CYCLE_DATA, first);
    put32(&mut rec, u, cycle);
    for bb in hdrs as usize + 1..len_bb {
        put32(&mut rec, bb * BBSIZE, cycle);
    }
    rec
}
