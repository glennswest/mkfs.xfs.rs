//! `xfs_admin -U` and `xfs_admin -L`: give an existing, unmounted
//! filesystem a new UUID or label over any [`BlockDevice`], leaving the
//! device byte for byte as `xfs_admin` (xfsprogs 6.15.0, `xfs_db -x -c
//! 'uuid …'` / `-c 'label …'`, `db/sb.c`) leaves it.
//!
//! ```no_run
//! # async fn f() -> mkfs_xfs::Result<()> {
//! use mkfs_xfs::{admin, device::FileDevice};
//! let dev = FileDevice::open("clone.img").await?;
//! admin::set_uuid(&dev, *uuid::Uuid::new_v4().as_bytes()).await?;
//! admin::set_label(&dev, "data").await?;
//! # Ok(()) }
//! ```
//!
//! A v5 filesystem names its UUID in every metadata block, so a new UUID
//! does not rewrite them: the old one stays in `sb_meta_uuid` and the
//! `META_UUID` incompat feature is set (`do_uuid`). Setting the UUID back
//! to the metadata UUID clears both again. The log record headers carry the
//! UUID too, so — as `xfs_admin` does — [`set_uuid`] first checks the log
//! is clean (`xlog_is_dirty`, refused with [`Error::NeedsRecovery`] if
//! not), then rewrites the **whole log** under the new UUID at the next
//! cycle (`libxfs_log_clear`). That is `sb_logblocks` of writes (64 MiB
//! for a filesystem under 128 GiB, up to 2 GiB), and on a thin device it
//! allocates the log. [`set_label`] writes only the superblocks.
//!
//! Every superblock is rewritten the way `xfs_db` rewrites it — decoded
//! and encoded again (`libxfs_sb_from_disk` / `libxfs_sb_to_disk`) — which
//! also turns a quota inode of 0 into `NULLFSINO` and copies
//! `sb_features2` into `sb_bad_features2`.
//!
//! Refused: a filesystem marked `NEEDSREPAIR` ([`Error::NeedsRecovery`],
//! as `xfs_db` refuses it), and — [`Error::Unsupported`] — one with an
//! external log, a realtime device, or an incompat feature this crate does
//! not know.

use crate::bytes::*;
use crate::crc;
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::inspect::{self, Geo};
use crate::structs::log::{self, BBSIZE, HEADER_CYCLE_SIZE, HEADER_MAGIC, UNMOUNT_TRANS};
use crate::structs::sb::{self, off, version};
use crate::structs::NULL64;

/// `XFSLABEL_MAX`: the label's length in the superblock.
pub const LABEL_MAX: usize = 12;

/// The incompat features whose superblock this module rewrites exactly.
const KNOWN_INCOMPAT: u32 = version::IN_FTYPE
    | version::IN_SPINODES
    | version::IN_META_UUID
    | version::IN_BIGTIME
    | version::IN_NEEDSREPAIR
    | version::IN_NREXT64;

/// `BDSTRAT_SIZE`: the record size of a full log clear.
const BDSTRAT_SIZE: u64 = 256 * 1024;
/// `XLOG_REC_SHIFT` for a v2 log, in basic blocks.
const REC_BBS: u64 = (1 << 18) / BBSIZE as u64;
/// `XLOG_TOTAL_REC_SHIFT` for a v2 log (`XLOG_MAX_ICLOGS` records).
const TOTAL_REC_BBS: u64 = (8 << 18) / BBSIZE as u64;
/// `XLOG_VERSION_2`.
const XLOG_VERSION_2: u32 = 2;
/// `XLOG_INIT_CYCLE`.
const INIT_CYCLE: u32 = 1;
/// How much of the log is read or written per device operation.
const WINDOW: u64 = 256 * 1024;
const CHUNK: usize = 4 << 20;

/// Give the filesystem the UUID `uuid`, as `xfs_admin -U` does: check the
/// log is clean, rewrite it under the new UUID, then rewrite every
/// allocation group's superblock (primary first). Flushes before
/// returning.
pub async fn set_uuid<D: BlockDevice + ?Sized>(dev: &D, uuid: [u8; 16]) -> Result<()> {
    let fs = Fs::open(dev).await?;

    // sb_logzero: refuse a dirty log, then clear it at the next cycle.
    let state = LogReader::new(dev, &fs)?.find_tail().await?;
    if state.dirty() {
        return Err(Error::NeedsRecovery(format!(
            "the log holds changes to replay (head {}, tail {}): mount and unmount it first",
            state.head, state.tail
        )));
    }
    let cycle = state.cycle + 1;
    fs.clear_log(&uuid, cycle).await?;
    dev.flush().await?;

    // do_uuid in each AG, against the primary's metadata UUID.
    let meta = fs.meta_uuid();
    for agno in 0..fs.geo.agcount {
        let mut b = fs.read_sb(agno).await?;
        let incompat = be32(&b, off::FEATURES_INCOMPAT);
        let has_meta = incompat & version::IN_META_UUID != 0;
        let mut incompat_new = incompat;
        if !has_meta && uuid != meta {
            incompat_new |= version::IN_META_UUID;
            b.copy_within(off::UUID..off::UUID + 16, off::META_UUID);
        } else if has_meta && uuid == meta {
            b[off::META_UUID..off::META_UUID + 16].fill(0);
            incompat_new &= !version::IN_META_UUID;
        }
        put32(&mut b, off::FEATURES_INCOMPAT, incompat_new);
        b[off::UUID..off::UUID + 16].copy_from_slice(&uuid);
        fs.write_sb(agno, b).await?;
    }
    dev.flush().await
}

/// Give the filesystem back the UUID its metadata carries, as `xfs_admin
/// -U restore` does: [`set_uuid`] with `sb_meta_uuid`, which clears
/// `META_UUID`. Returns `false`, writing nothing, when the UUID was never
/// changed.
pub async fn restore_uuid<D: BlockDevice + ?Sized>(dev: &D) -> Result<bool> {
    let fs = Fs::open(dev).await?;
    if be32(&fs.primary, off::FEATURES_INCOMPAT) & version::IN_META_UUID == 0 {
        return Ok(false);
    }
    set_uuid(dev, fs.meta_uuid()).await?;
    Ok(true)
}

/// Set the filesystem label (`sb_fname`) in every superblock, as
/// `xfs_admin -L` does. An empty label clears it.
///
/// `xfs_admin` truncates a label over [`LABEL_MAX`] bytes, and reads `--`
/// as "clear"; both are the command line's business, so here a longer
/// label (or one holding a NUL) is refused with [`Error::InvalidParams`]
/// and `--` is a label like any other.
pub async fn set_label<D: BlockDevice + ?Sized>(dev: &D, label: &str) -> Result<()> {
    if label.len() > LABEL_MAX || label.contains('\0') {
        return Err(Error::InvalidParams(format!(
            "an XFS label is at most {LABEL_MAX} bytes with no NUL: {label:?}"
        )));
    }
    let fs = Fs::open(dev).await?;
    let mut fname = [0u8; LABEL_MAX];
    fname[..label.len()].copy_from_slice(label.as_bytes());
    for agno in 0..fs.geo.agcount {
        let mut b = fs.read_sb(agno).await?;
        b[off::FNAME..off::FNAME + LABEL_MAX].copy_from_slice(&fname);
        fs.write_sb(agno, b).await?;
    }
    dev.flush().await
}

/// Where the log's head and tail are and which cycle it is in, as
/// `xlog_find_tail` (`libxlog`) finds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogState {
    /// Basic block the next record would be written at.
    pub head: u64,
    /// Basic block of the oldest record still needed.
    pub tail: u64,
    /// `l_curr_cycle`: the cycle the head is in (0 for a zeroed log).
    pub cycle: u32,
}

impl LogState {
    /// Whether the log holds changes not yet in place (`xlog_is_dirty`).
    pub fn dirty(&self) -> bool {
        self.head != self.tail
    }
}

/// Find the internal log's head, tail and cycle.
pub async fn log_state<D: BlockDevice + ?Sized>(dev: &D) -> Result<LogState> {
    let fs = Fs::open(dev).await?;
    let mut log = LogReader::new(dev, &fs)?;
    log.find_tail().await
}

/// The primary superblock, read and checked.
struct Fs<'a, D: ?Sized> {
    dev: &'a D,
    geo: Geo,
    primary: Vec<u8>,
}

impl<'a, D: BlockDevice + ?Sized> Fs<'a, D> {
    async fn open(dev: &'a D) -> Result<Self> {
        let geo = inspect::geometry(dev).await?;
        let mut fs = Fs { dev, geo, primary: Vec::new() };
        let p = fs.read_sb(0).await?;
        let incompat = be32(&p, off::FEATURES_INCOMPAT);
        if incompat & version::IN_NEEDSREPAIR != 0 {
            return Err(Error::NeedsRecovery("marked as needing xfs_repair".into()));
        }
        if incompat & !KNOWN_INCOMPAT != 0 {
            return Err(Error::Unsupported(format!(
                "incompat features {:#x}",
                incompat & !KNOWN_INCOMPAT
            )));
        }
        if be64(&p, off::RBLOCKS) != 0 {
            return Err(Error::Unsupported("a realtime device".into()));
        }
        if fs.geo.logstart == 0 {
            return Err(Error::Unsupported("an external log".into()));
        }
        fs.primary = p;
        Ok(fs)
    }

    fn sb_offset(&self, agno: u64) -> u64 {
        agno * self.geo.agblocks * self.geo.blocksize
    }

    /// AG `agno`'s superblock block, its magic and CRC checked.
    async fn read_sb(&self, agno: u64) -> Result<Vec<u8>> {
        let mut b = vec![0u8; self.geo.blocksize as usize];
        self.dev.read_at(self.sb_offset(agno), &mut b).await?;
        if be32(&b, off::MAGICNUM) != sb::MAGIC {
            return Err(Error::Corrupt(format!("AG {agno} superblock: no XFSB magic")));
        }
        if !crc::verify(&b[..self.geo.sectsize as usize], off::CRC) {
            return Err(Error::Corrupt(format!("AG {agno} superblock: bad CRC")));
        }
        Ok(b)
    }

    /// Write a superblock back as `libxfs_sb_to_disk` and the write
    /// verifier leave it.
    async fn write_sb(&self, agno: u64, mut b: Vec<u8>) -> Result<()> {
        // xfs_sb_quota_from_disk: 0 is read as NULLFSINO, and written so.
        for o in [off::UQUOTINO, off::GQUOTINO, off::PQUOTINO] {
            if be64(&b, o) == 0 {
                put64(&mut b, o, NULL64);
            }
        }
        // xfs_sb_to_disk: bad_features2 always matches features2.
        let f2 = be32(&b, off::FEATURES2);
        put32(&mut b, off::BAD_FEATURES2, f2);
        crc::stamp(&mut b[..self.geo.sectsize as usize], off::CRC);
        self.dev.write_at(self.sb_offset(agno), &b).await
    }

    /// `mp->m_sb.sb_meta_uuid`: the on-disk metadata UUID with
    /// `META_UUID`, otherwise `sb_uuid`.
    fn meta_uuid(&self) -> [u8; 16] {
        let p = &self.primary;
        let o = if be32(p, off::FEATURES_INCOMPAT) & version::IN_META_UUID != 0 {
            off::META_UUID
        } else {
            off::UUID
        };
        p[o..o + 16].try_into().unwrap()
    }

    /// Byte offset and length in basic blocks of the internal log
    /// (`XFS_FSB_TO_DADDR(sb_logstart)`, `XFS_FSB_TO_BB(sb_logblocks)`).
    fn log_extent(&self) -> (u64, u64) {
        let g = &self.geo;
        let agno = g.logstart >> g.agblklog;
        let agbno = g.logstart & ((1u64 << g.agblklog) - 1);
        let start = (agno * g.agblocks + agbno) * g.blocksize;
        (start, g.logblocks * g.blocksize / BBSIZE as u64)
    }

    /// `libxfs_log_clear` with `max`, as `sb_logzero` calls it: the log
    /// zeroed, one record at block 0 in `cycle` whose tail points at the
    /// end of the previous cycle, and — past `XLOG_INIT_CYCLE` — the rest
    /// filled with `BDSTRAT_SIZE` records of the previous cycle, from
    /// basic block `BTOBB(BDSTRAT_SIZE)` on.
    async fn clear_log(&self, uuid: &[u8; 16], cycle: u32) -> Result<()> {
        let (start, bbs) = self.log_extent();
        // sb_logzero passes sb_logsunit as it is: 1 (none) is one byte.
        let sunit = self.geo.logsunit;
        let len = if sunit > 0 { u64::from(sunit).div_ceil(BBSIZE as u64) } else { 2 }.max(2);
        let lsn = log::lsn(cycle, 0);
        let tail = if cycle == INIT_CYCLE { lsn } else { log::lsn(cycle - 1, (bbs - len) as u32) };
        let first = log::record(uuid, sunit, lsn, tail);
        if first.len() as u64 != len * BBSIZE as u64 {
            return Err(Error::Unsupported(format!("log stripe unit {sunit}")));
        }

        let mut out = LogWriter { dev: self.dev, start, at: 0, buf: Vec::with_capacity(CHUNK) };
        if cycle == INIT_CYCLE {
            self.dev.write_zeroes(start, bbs * BBSIZE as u64).await?;
            out.push(&first).await?;
            out.pad_to_block(self.geo.blocksize as usize);
            return out.finish().await;
        }
        out.push(&first).await?;
        let c = cycle - 1;
        let max = BDSTRAT_SIZE / BBSIZE as u64;
        let mut blk = max;
        let mut rlen = bbs.saturating_sub(blk).min(max);
        out.zeroes_to(blk.min(bbs) * BBSIZE as u64).await?;
        while blk < bbs {
            let rec = log::record(
                uuid,
                (rlen * BBSIZE as u64) as u32,
                log::lsn(c, blk as u32),
                log::lsn(c, blk.wrapping_sub(rlen) as u32),
            );
            if rec.len() as u64 != rlen * BBSIZE as u64 {
                return Err(Error::Unsupported(format!("a log record of {rlen} basic blocks")));
            }
            out.push(&rec).await?;
            blk += rlen;
            rlen = (bbs - blk).min(rlen);
        }
        out.finish().await
    }
}

/// Writes the log front to back in whole chunks.
struct LogWriter<'a, D: ?Sized> {
    dev: &'a D,
    start: u64,
    /// Log offset of `buf[0]`.
    at: u64,
    buf: Vec<u8>,
}

impl<D: BlockDevice + ?Sized> LogWriter<'_, D> {
    async fn push(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend_from_slice(data);
        while self.buf.len() >= CHUNK {
            self.dev.write_at(self.start + self.at, &self.buf[..CHUNK]).await?;
            self.buf.drain(..CHUNK);
            self.at += CHUNK as u64;
        }
        Ok(())
    }

    async fn zeroes_to(&mut self, end: u64) -> Result<()> {
        while self.at + (self.buf.len() as u64) < end {
            let n = (end - self.at - self.buf.len() as u64).min(CHUNK as u64) as usize;
            self.push(&vec![0u8; n]).await?;
        }
        Ok(())
    }

    fn pad_to_block(&mut self, blocksize: usize) {
        let n = self.buf.len().next_multiple_of(blocksize);
        self.buf.resize(n, 0);
    }

    async fn finish(self) -> Result<()> {
        if !self.buf.is_empty() {
            self.dev.write_at(self.start + self.at, &self.buf).await?;
        }
        Ok(())
    }
}

/// The log read a basic block at a time, through whole-block windows.
struct LogReader<'a, D: ?Sized> {
    dev: &'a D,
    start: u64,
    /// `l_logBBsize`.
    bbs: u64,
    win_at: u64,
    win: Vec<u8>,
}

/// The outcome of `xlog_find_verify_log_record` that is not an error.
enum Found {
    Header,
    /// Reached block 0 with no header (the C code's -1).
    None,
}

impl<'a, D: BlockDevice + ?Sized> LogReader<'a, D> {
    fn new(dev: &'a D, fs: &Fs<'a, D>) -> Result<Self> {
        let (start, bbs) = fs.log_extent();
        if bbs < TOTAL_REC_BBS {
            return Err(Error::Unsupported(format!("a log of {bbs} basic blocks")));
        }
        Ok(LogReader { dev, start, bbs, win_at: 0, win: Vec::new() })
    }

    /// Basic block `n` of the log.
    async fn bb(&mut self, n: u64) -> Result<&[u8]> {
        let byte = n * BBSIZE as u64;
        if self.win.is_empty() || byte < self.win_at || byte >= self.win_at + self.win.len() as u64 {
            let at = byte / WINDOW * WINDOW;
            let len = WINDOW.min(self.bbs * BBSIZE as u64 - at) as usize;
            self.win.resize(len, 0);
            self.dev.read_at(self.start + at, &mut self.win).await?;
            self.win_at = at;
        }
        let o = (byte - self.win_at) as usize;
        Ok(&self.win[o..o + BBSIZE])
    }

    /// `xlog_get_cycle`.
    async fn cycle(&mut self, n: u64) -> Result<u32> {
        let b = self.bb(n).await?;
        Ok(if be32(b, 0) == HEADER_MAGIC { be32(b, log::off::CYCLE) } else { be32(b, 0) })
    }

    async fn is_header(&mut self, n: u64) -> Result<bool> {
        Ok(be32(self.bb(n).await?, 0) == HEADER_MAGIC)
    }

    /// `xlog_find_cycle_start`: binary search for the first block in
    /// `(first, last]` with `cycle`.
    async fn find_cycle_start(&mut self, mut first: u64, last: u64, cycle: u32) -> Result<u64> {
        let mut end = last;
        let mut mid = (first + end) / 2;
        while mid != first && mid != end {
            if self.cycle(mid).await? == cycle {
                end = mid;
            } else {
                first = mid;
            }
            mid = (first + end) / 2;
        }
        Ok(end)
    }

    /// `xlog_find_verify_cycle`: the first block in `start..start+n` with
    /// cycle `stop`.
    async fn find_verify_cycle(&mut self, start: u64, n: u64, stop: u32) -> Result<Option<u64>> {
        for i in start..start + n {
            if self.cycle(i).await? == stop {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// `xlog_find_verify_log_record`: back `last` up over a partly
    /// written record.
    async fn find_verify_log_record(&mut self, start: u64, last: &mut u64, extra: u64) -> Result<Found> {
        let mut found = None;
        for i in (0..*last).rev() {
            if i < start {
                return Err(Error::Corrupt("log inconsistent (didn't find previous header)".into()));
            }
            if self.is_header(i).await? {
                found = Some(i);
                break;
            }
        }
        let Some(i) = found else { return Ok(Found::None) };
        let h = self.bb(i).await?;
        let h_size = be32(h, log::off::SIZE);
        let xhdrs = u64::from(h_size.div_ceil(HEADER_CYCLE_SIZE));
        let h_len = u64::from(be32(h, log::off::LEN)).div_ceil(BBSIZE as u64);
        if *last - i + extra != h_len + xhdrs {
            *last = i;
        }
        Ok(Found::Header)
    }

    /// `xlog_find_zeroed`: `Some(first zero block)` if the log is not
    /// written all the way round, `None` if it is.
    async fn find_zeroed(&mut self) -> Result<Option<u64>> {
        let first = self.cycle(0).await?;
        if first == 0 {
            return Ok(Some(0));
        }
        if self.cycle(self.bbs - 1).await? != 0 {
            return Ok(None);
        }
        if first != 1 {
            return Err(Error::Corrupt("log inconsistent or not a log (last==0, first!=1)".into()));
        }
        let mut last = self.find_cycle_start(0, self.bbs - 1, 0).await?;
        let n = TOTAL_REC_BBS.min(last);
        let start = last - n;
        if let Some(b) = self.find_verify_cycle(start, n, 0).await? {
            last = b;
        }
        match self.find_verify_log_record(start, &mut last, 0).await? {
            Found::Header => Ok(Some(last)),
            Found::None => Err(Error::Corrupt("log: no record before the zeroed blocks".into())),
        }
    }

    /// `xlog_find_head`.
    async fn find_head(&mut self) -> Result<u64> {
        if let Some(b) = self.find_zeroed().await? {
            return Ok(b);
        }
        let n = self.bbs;
        let first_half = self.cycle(0).await?;
        let last_half = self.cycle(n - 1).await?;
        let mut head;
        let stop;
        if first_half == last_half {
            head = n;
            stop = last_half.wrapping_sub(1);
        } else {
            stop = last_half;
            head = self.find_cycle_start(0, n - 1, last_half).await?;
        }

        let mut validate = false;
        if head >= TOTAL_REC_BBS {
            if let Some(b) = self.find_verify_cycle(head - TOTAL_REC_BBS, TOTAL_REC_BBS, stop).await? {
                head = b;
            }
        } else {
            let start = n - (TOTAL_REC_BBS - head);
            if let Some(b) = self.find_verify_cycle(start, TOTAL_REC_BBS - head, stop.wrapping_sub(1)).await? {
                head = b;
                validate = true;
            }
            if !validate {
                if let Some(b) = self.find_verify_cycle(0, head, stop).await? {
                    head = b;
                }
            }
        }

        // validate_head: not in the middle of a record.
        if head >= REC_BBS {
            if let Found::None = self.find_verify_log_record(head - REC_BBS, &mut head, 0).await? {
                return Err(Error::Corrupt("log: no record header before the head".into()));
            }
        } else if let Found::None = self.find_verify_log_record(0, &mut head, 0).await? {
            let start = n - (REC_BBS - head);
            let mut new = n;
            if let Found::None = self.find_verify_log_record(start, &mut new, head).await? {
                return Err(Error::Corrupt("log: no record header before the head".into()));
            }
            if new != n {
                head = new;
            }
        }
        Ok(if head == n { 0 } else { head })
    }

    /// `xlog_find_tail`, as far as `xlog_is_dirty` and `sb_logzero` use it.
    async fn find_tail(&mut self) -> Result<LogState> {
        let head = self.find_head().await?;
        if head == 0 && self.cycle(0).await? == 0 {
            return Ok(LogState { head: 0, tail: 0, cycle: 0 });
        }
        let mut found = None;
        for i in (0..head).rev() {
            if self.is_header(i).await? {
                found = Some((i, false));
                break;
            }
        }
        if found.is_none() {
            for i in (head..self.bbs).rev() {
                if self.is_header(i).await? {
                    found = Some((i, true));
                    break;
                }
            }
        }
        let Some((i, wrapped)) = found else {
            return Err(Error::Corrupt("log: couldn't find sync record".into()));
        };
        let h = self.bb(i).await?;
        let mut tail = be64(h, log::off::TAIL_LSN) & 0xffff_ffff;
        let mut cycle = be32(h, log::off::CYCLE);
        if wrapped {
            cycle += 1;
        }
        let h_size = be32(h, log::off::SIZE);
        let h_version = be32(h, log::off::VERSION);
        let hblks = if h_version & XLOG_VERSION_2 != 0 && h_size > HEADER_CYCLE_SIZE {
            u64::from(h_size.div_ceil(HEADER_CYCLE_SIZE))
        } else {
            1
        };
        let h_len = u64::from(be32(h, log::off::LEN)).div_ceil(BBSIZE as u64);
        let num_logops = be32(h, log::off::NUM_LOGOPS);
        let after_umount = (i + hblks + h_len) % self.bbs;
        if head == after_umount && num_logops == 1 {
            // xlog_op_header: oh_flags is the byte after oh_clientid.
            let op = self.bb((i + hblks) % self.bbs).await?;
            if op[9] & UNMOUNT_TRANS != 0 {
                tail = after_umount;
            }
        }
        Ok(LogState { head, tail, cycle })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::MemDevice;
    use crate::format::format;
    use crate::geometry::Params;

    const OLD: [u8; 16] = [1; 16];
    const NEW: [u8; 16] = [2; 16];

    async fn fresh(size: u64, p: Params) -> MemDevice {
        let dev = MemDevice::new(size);
        format(&dev, &p.uuid(OLD)).await.unwrap();
        dev
    }

    async fn sb_of(dev: &MemDevice, agno: u64) -> Vec<u8> {
        let fs = Fs::open(dev).await.unwrap();
        fs.read_sb(agno).await.unwrap()
    }

    #[tokio::test]
    async fn fresh_log_is_clean_at_cycle_1() {
        let dev = fresh(1 << 30, Params::new()).await;
        let s = log_state(&dev).await.unwrap();
        assert_eq!(s, LogState { head: 2, tail: 2, cycle: 1 });
    }

    #[tokio::test]
    async fn new_uuid_keeps_meta_uuid_and_moves_the_log_on() {
        let dev = fresh(1 << 30, Params::new()).await;
        set_uuid(&dev, NEW).await.unwrap();
        let agcount = Fs::open(&dev).await.unwrap().geo.agcount;
        for agno in 0..agcount {
            let b = sb_of(&dev, agno).await;
            assert_eq!(b[off::UUID..off::UUID + 16], NEW);
            assert_eq!(b[off::META_UUID..off::META_UUID + 16], OLD);
            assert_ne!(be32(&b, off::FEATURES_INCOMPAT) & version::IN_META_UUID, 0);
            assert_eq!(be64(&b, off::UQUOTINO), NULL64);
        }
        let s = log_state(&dev).await.unwrap();
        assert!(!s.dirty());
        assert_eq!(s.cycle, 2);

        // Again: the log is now written all the way round.
        set_uuid(&dev, [3; 16]).await.unwrap();
        let s = log_state(&dev).await.unwrap();
        assert!(!s.dirty());
        assert_eq!(s.cycle, 3);
        let b = sb_of(&dev, 0).await;
        assert_eq!(b[off::META_UUID..off::META_UUID + 16], OLD);

        // Back to the metadata UUID: META_UUID goes, and the field is zeroed.
        set_uuid(&dev, OLD).await.unwrap();
        for agno in 0..agcount {
            let b = sb_of(&dev, agno).await;
            assert_eq!(b[off::UUID..off::UUID + 16], OLD);
            assert_eq!(b[off::META_UUID..off::META_UUID + 16], [0; 16]);
            assert_eq!(be32(&b, off::FEATURES_INCOMPAT) & version::IN_META_UUID, 0);
        }
        assert_eq!(log_state(&dev).await.unwrap().cycle, 4);
    }

    #[tokio::test]
    async fn dirty_log_is_refused() {
        let dev = fresh(1 << 30, Params::new()).await;
        // Turn the unmount record into an ordinary transaction.
        let fs = Fs::open(&dev).await.unwrap();
        let (start, _) = fs.log_extent();
        let mut blk = vec![0u8; fs.geo.blocksize as usize];
        dev.read_at(start, &mut blk).await.unwrap();
        blk[BBSIZE + 9] = 0;
        dev.write_at(start, &blk).await.unwrap();
        assert!(log_state(&dev).await.unwrap().dirty());
        let before = sb_of(&dev, 0).await;
        assert!(matches!(set_uuid(&dev, NEW).await, Err(Error::NeedsRecovery(_))));
        assert_eq!(sb_of(&dev, 0).await, before, "nothing written");
    }

    #[tokio::test]
    async fn label_set_and_cleared() {
        let dev = fresh(1 << 30, Params::new()).await;
        set_label(&dev, "twelve-chars").await.unwrap();
        let agcount = Fs::open(&dev).await.unwrap().geo.agcount;
        for agno in 0..agcount {
            assert_eq!(&sb_of(&dev, agno).await[off::FNAME..off::FNAME + 12], b"twelve-chars");
        }
        set_label(&dev, "").await.unwrap();
        assert_eq!(sb_of(&dev, 0).await[off::FNAME..off::FNAME + 12], [0; 12]);
        assert!(matches!(set_label(&dev, "thirteen-char").await, Err(Error::InvalidParams(_))));
        // The log is untouched.
        assert_eq!(log_state(&dev).await.unwrap().cycle, 1);
    }

    #[tokio::test]
    async fn needsrepair_is_refused() {
        let dev = fresh(1 << 30, Params::new()).await;
        let mut b = sb_of(&dev, 0).await;
        let f = be32(&b, off::FEATURES_INCOMPAT);
        put32(&mut b, off::FEATURES_INCOMPAT, f | version::IN_NEEDSREPAIR);
        crc::stamp(&mut b[..512], off::CRC);
        dev.write_at(0, &b).await.unwrap();
        assert!(matches!(set_uuid(&dev, NEW).await, Err(Error::NeedsRecovery(_))));
        assert!(matches!(set_label(&dev, "x").await, Err(Error::NeedsRecovery(_))));
    }
}
