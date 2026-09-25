//! The formatter: lays down what `mkfs.xfs` would leave on the device.
//!
//! `mkfs.xfs` gets there by writing AG headers, then running libxfs
//! transactions — fill each AGFL, allocate the root inode chunk, create the
//! root directory and the realtime bitmap and summary inodes — and writing
//! back whatever those transactions left. This crate computes that end state
//! directly ([`plan_ag`]) and writes it once, every allocation group in
//! parallel. The golden tests hold the result to real `mkfs.xfs` output.
//!
//! Order of writes, so an interrupted format is recognisably unfinished:
//! the device's first and last 128 KiB are zeroed (old signatures), the log
//! is zeroed and given its unmount record, every AG is written with the
//! primary superblock still marked `sb_inprogress`, the device is flushed,
//! and only then is the primary superblock rewritten as finished.

use futures::stream::{self, StreamExt, TryStreamExt};

use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::geometry::{Geometry, Params, Timestamp};
use crate::structs::ag::{encode_agfl, Agf, Agi};
use crate::structs::btree::{self, owner, AllocRec, InobtRec, RmapRec};
use crate::structs::inode::{self, Dinode};
use crate::structs::{log, sb, NULL32, NULL64};

/// `WHACK_SIZE`: how much `mkfs.xfs` zeroes at each end of the device.
pub const WHACK_SIZE: u64 = 128 * 1024;

/// How many allocation groups are written at once.
const AG_CONCURRENCY: usize = 32;

/// What a format produced.
#[derive(Debug, Clone)]
pub struct Report {
    /// The geometry written.
    pub geometry: Geometry,
    /// The filesystem UUID.
    pub uuid: [u8; 16],
    /// The root directory's inode.
    pub rootino: u64,
    /// Free data blocks, as the primary superblock records them.
    pub fdblocks: u64,
}

/// The end state of one allocation group's metadata.
#[derive(Debug, Clone)]
pub struct AgPlan {
    /// AG number.
    pub agno: u64,
    /// Blocks in this AG.
    pub length: u64,
    /// Free extents, by block.
    pub free: Vec<AllocRec>,
    /// Blocks on the free list, in AGFL order.
    pub agfl: Vec<u32>,
    /// Reverse mappings, by block.
    pub rmap: Vec<RmapRec>,
    /// The root inode chunk's first block, in AG 0.
    pub chunk: Option<u64>,
}

impl AgPlan {
    /// Free blocks in the free space btrees.
    pub fn freeblks(&self) -> u64 {
        self.free.iter().map(|r| u64::from(r.count)).sum()
    }

    /// The longest free extent.
    pub fn longest(&self) -> u64 {
        self.free.iter().map(|r| u64::from(r.count)).max().unwrap_or(0)
    }
}

/// Insert a reverse mapping the way `xfs_rmap_map` does for a special
/// owner: merged with a contiguous neighbour of the same owner.
fn rmap_insert(rmap: &mut Vec<RmapRec>, rec: RmapRec) {
    let pos = rmap.partition_point(|r| r.start < rec.start);
    let left = pos.checked_sub(1).filter(|&i| {
        let l = rmap[i];
        l.owner == rec.owner && l.start + l.count == rec.start
    });
    let right = Some(pos).filter(|&i| {
        i < rmap.len() && rmap[i].owner == rec.owner && rec.start + rec.count == rmap[i].start
    });
    match (left, right) {
        (Some(l), Some(r)) => {
            rmap[l].count += rec.count + rmap[r].count;
            rmap.remove(r);
        }
        (Some(l), None) => rmap[l].count += rec.count,
        (None, Some(r)) => {
            rmap[r].start = rec.start;
            rmap[r].count += rec.count;
        }
        (None, None) => rmap.insert(pos, rec),
    }
}

/// Take `[start, start + len)` out of the free extents.
fn take(free: &mut Vec<AllocRec>, start: u64, len: u64) -> Result<()> {
    let i = free
        .iter()
        .position(|r| {
            u64::from(r.start) <= start && start + len <= u64::from(r.start) + u64::from(r.count)
        })
        .ok_or_else(|| Error::InvalidParams(format!("blocks {start}+{len} are not free")))?;
    let r = free.remove(i);
    let (rs, re) = (u64::from(r.start), u64::from(r.start) + u64::from(r.count));
    let mut at = i;
    if start > rs {
        free.insert(at, AllocRec { start: rs as u32, count: (start - rs) as u32 });
        at += 1;
    }
    if start + len < re {
        free.insert(at, AllocRec { start: (start + len) as u32, count: (re - start - len) as u32 });
    }
    Ok(())
}

/// The end state of allocation group `agno` after `mkfs.xfs`'s header
/// initialisation (`xfs_ag_init_headers`), freelist fill
/// (`xfs_alloc_fix_freelist`) and, in AG 0, root inode chunk allocation.
pub fn plan_ag(g: &Geometry, agno: u64) -> Result<AgPlan> {
    let length = g.ag_blocks(agno);
    let p = g.prealloc_blocks();
    let f = g.features;

    // xfs_freesp_init_recs: everything after the preallocated blocks is
    // free, less the log in its AG.
    let mut free = Vec::new();
    let mut rmap = Vec::new();
    // xfs_rmaproot_init: the static records, written as they are.
    if f.rmapbt {
        let rec = |start: u64, count: u64, owner: u64| RmapRec {
            start: start as u32,
            count: count as u32,
            owner,
        };
        rmap.push(rec(0, g.bno_block(), owner::FS));
        rmap.push(rec(g.bno_block(), 2, owner::AG));
        rmap.push(rec(g.ibt_block(), g.rmap_block() - g.ibt_block(), owner::INOBT));
        rmap.push(rec(g.rmap_block(), 1, owner::AG));
        if f.reflink {
            rmap.push(rec(g.refc_block(), 1, owner::REFC));
        }
    }
    if agno == g.logagno {
        let (ls, le) = (g.log_agbno, g.log_agbno + g.logblocks);
        if ls > p {
            free.push(AllocRec { start: p as u32, count: (ls - p) as u32 });
        }
        if le < length {
            free.push(AllocRec { start: le as u32, count: (length - le) as u32 });
        }
        if f.rmapbt {
            rmap.push(RmapRec { start: ls as u32, count: g.logblocks as u32, owner: owner::LOG });
        }
    } else {
        free.push(AllocRec { start: p as u32, count: (length - p) as u32 });
    }

    // xfs_alloc_fix_freelist: allocate by size — the smallest extent that
    // holds what is still needed, from its start — until the list is full.
    let need = g.min_freelist();
    let mut agfl = Vec::new();
    while (agfl.len() as u64) < need {
        let want = need - agfl.len() as u64;
        let pick = free
            .iter()
            .filter(|r| u64::from(r.count) >= want)
            .min_by_key(|r| (r.count, r.start))
            .or_else(|| free.iter().max_by_key(|r| (r.count, std::cmp::Reverse(r.start))))
            .copied()
            .ok_or_else(|| Error::InvalidParams(format!("AG {agno} has no space for its free list")))?;
        let len = want.min(u64::from(pick.count));
        take(&mut free, u64::from(pick.start), len)?;
        agfl.extend((0..len).map(|i| pick.start + i as u32));
        if f.rmapbt {
            rmap_insert(&mut rmap, RmapRec { start: pick.start, count: len as u32, owner: owner::AG });
        }
    }

    // The root inode chunk, where xfs_ialloc_calc_rootino says it must be.
    let chunk = if agno == 0 {
        let at = g.root_chunk_agbno();
        take(&mut free, at, g.chunk_blocks()).map_err(|_| {
            Error::InvalidParams(format!("the root inode chunk at block {at} of AG 0 is not free"))
        })?;
        if f.rmapbt {
            rmap_insert(
                &mut rmap,
                RmapRec { start: at as u32, count: g.chunk_blocks() as u32, owner: owner::INODES },
            );
        }
        Some(at)
    } else {
        None
    };

    Ok(AgPlan { agno, length, free, agfl, rmap, chunk })
}

/// A small, fast generator for inode generation numbers (splitmix64).
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as u32
    }
}

/// The inodes of the root chunk: every slot initialised free, then the
/// root directory and the realtime bitmap and summary inodes.
fn root_chunk(g: &Geometry, agbno: u64, uuid: &[u8; 16], time: Timestamp, gen: &mut Gen) -> Vec<u8> {
    let isz = g.inodesize as usize;
    let mut buf = vec![0u8; (g.chunk_blocks() * g.bs()) as usize];
    let first = g.ino(0, agbno, 0);
    // xfs_ialloc_inode_init: one generation number for the whole chunk.
    let chunk_gen = gen.next();
    for (i, slot) in buf.chunks_mut(isz).enumerate() {
        inode::encode_free(first + i as u64, chunk_gen, uuid, slot);
    }

    let mut flags2 = 0;
    if g.features.bigtime {
        flags2 |= inode::DIFLAG2_BIGTIME;
    }
    if g.features.nrext64 {
        flags2 |= inode::DIFLAG2_NREXT64;
    }
    let base = Dinode {
        ino: first,
        mode: 0,
        format: inode::format::EXTENTS,
        nlink: 1,
        size: 0,
        flags: 0,
        flags2,
        // xfs_icreate draws a new one; the root's is zeroed (xfsdump).
        gen: 0,
        // One for the create, one for the transaction that logged it.
        changecount: 2,
        atime: time,
        mtime: time,
        ctime: time,
        crtime: time,
        data: Vec::new(),
    };
    let root = Dinode {
        mode: 0o040755,
        format: inode::format::LOCAL,
        nlink: 2,
        data: inode::empty_sf_dir(first),
        ..base.clone()
    };
    root.size_from_data().encode(uuid, &mut buf[..isz]);
    // xfs_rtbitmap_create: its atime is the summary's sequence counter.
    let rbm = Dinode {
        ino: first + 1,
        mode: 0o100000,
        flags: inode::DIFLAG_NEWRTBM,
        gen: gen.next(),
        atime: Timestamp { secs: 0, nsecs: 0 },
        ..base.clone()
    };
    rbm.encode(uuid, &mut buf[isz..2 * isz]);
    let rsum = Dinode { ino: first + 2, mode: 0o100000, gen: gen.next(), ..base };
    rsum.encode(uuid, &mut buf[2 * isz..3 * isz]);
    buf
}

impl Dinode {
    /// Set the size from the local data fork (a short-form directory).
    fn size_from_data(mut self) -> Self {
        self.size = self.data.len() as u64;
        self
    }
}

/// The header sectors and btree roots of one AG: blocks `0..prealloc`.
fn ag_head(g: &Geometry, plan: &AgPlan, uuid: &[u8; 16], sbs: &sb::SbState) -> Vec<u8> {
    let bs = g.bs() as usize;
    let ss = g.sectsize as usize;
    let f = g.features;
    let p = g.prealloc_blocks() as usize;
    let agno = plan.agno;
    let mut buf = vec![0u8; p * bs];

    sb::encode(g, sbs, &mut buf[..ss]);

    let agfl_len = plan.agfl.len() as u32;
    let agf = Agf {
        seqno: agno as u32,
        length: plan.length as u32,
        bno_root: g.bno_block() as u32,
        bno_level: 1,
        cnt_root: g.cnt_block() as u32,
        cnt_level: 1,
        rmap_root: if f.rmapbt { g.rmap_block() as u32 } else { 0 },
        rmap_level: u32::from(f.rmapbt),
        rmap_blocks: u32::from(f.rmapbt),
        refcount_root: if f.reflink { g.refc_block() as u32 } else { 0 },
        refcount_level: u32::from(f.reflink),
        refcount_blocks: u32::from(f.reflink),
        // xfs_agfblock_init starts the list at slot 1; each put advances
        // fllast first.
        flfirst: 1,
        fllast: agfl_len % g.agfl_size(),
        flcount: agfl_len,
        freeblks: plan.freeblks() as u32,
        longest: plan.longest() as u32,
        btreeblks: 0,
    };
    agf.encode(uuid, &mut buf[ss..2 * ss]);

    let chunk = plan.chunk.map(|c| (c << g.inopblog) as u32);
    let (count, freecount) = if chunk.is_some() { (64, 61) } else { (0, 0) };
    let agi = Agi {
        seqno: agno as u32,
        length: plan.length as u32,
        count,
        root: g.ibt_block() as u32,
        level: 1,
        freecount,
        newino: chunk.unwrap_or(NULL32),
        free_root: if f.finobt { g.fibt_block() as u32 } else { 0 },
        free_level: u32::from(f.finobt),
        iblocks: u32::from(f.inobtcount),
        fblocks: u32::from(f.inobtcount && f.finobt),
    };
    agi.encode(uuid, &mut buf[2 * ss..3 * ss]);
    encode_agfl(agno as u32, uuid, 1, &plan.agfl, &mut buf[3 * ss..4 * ss]);

    // Btree roots, each one leaf block.
    let block = |agbno: u64| -> std::ops::Range<usize> {
        agbno as usize * bs..(agbno as usize + 1) * bs
    };
    let leaf = |magic: u32, agbno: u64, recs: &[u8], n: usize| {
        btree::leaf_block(magic, g.agb_to_daddr(agno, agbno), agno as u32, uuid, recs, n as u16, bs)
    };
    let mut by_size = plan.free.clone();
    by_size.sort_by_key(|r| (r.count, r.start));
    let bno = leaf(btree::magic::BNO, g.bno_block(), &btree::alloc_recs(&plan.free), plan.free.len());
    buf[block(g.bno_block())].copy_from_slice(&bno);
    let cnt = leaf(btree::magic::CNT, g.cnt_block(), &btree::alloc_recs(&by_size), by_size.len());
    buf[block(g.cnt_block())].copy_from_slice(&cnt);

    let inodes: Vec<InobtRec> = chunk
        .map(|startino| InobtRec {
            startino,
            holemask: 0,
            count: 64,
            freecount: 61,
            free: !0b111u64,
        })
        .into_iter()
        .collect();
    let recs = btree::inobt_recs(&inodes, f.sparse);
    buf[block(g.ibt_block())].copy_from_slice(&leaf(btree::magic::INO, g.ibt_block(), &recs, inodes.len()));
    if f.finobt {
        let fino = leaf(btree::magic::FINO, g.fibt_block(), &recs, inodes.len());
        buf[block(g.fibt_block())].copy_from_slice(&fino);
    }
    if f.rmapbt {
        let recs = btree::rmap_recs(&plan.rmap);
        let rmap = leaf(btree::magic::RMAP, g.rmap_block(), &recs, plan.rmap.len());
        buf[block(g.rmap_block())].copy_from_slice(&rmap);
    }
    if f.reflink {
        let refc = leaf(btree::magic::REFC, g.refc_block(), &[], 0);
        buf[block(g.refc_block())].copy_from_slice(&refc);
    }
    buf
}

/// Format `dev` as `mkfs.xfs` would with `params`.
pub async fn format<D: BlockDevice + ?Sized>(dev: &D, params: &Params) -> Result<Report> {
    let g = Geometry::compute(params, dev.size(), dev.logical_sector_size(), dev.physical_sector_size())?;
    let uuid = params.uuid.unwrap_or_else(|| *uuid::Uuid::new_v4().as_bytes());
    let time = params.time.unwrap_or_else(Timestamp::now);
    let mut gen = Gen(params.gen_seed.unwrap_or_else(|| {
        u64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().unwrap())
    }));
    let bs = g.bs();

    let plans = (0..g.agcount).map(|a| plan_ag(&g, a)).collect::<Result<Vec<_>>>()?;
    let rootino = g.rootino();
    let fdblocks: u64 = plans.iter().map(|p| p.freeblks() + p.agfl.len() as u64).sum::<u64>();
    let secondary = sb::SbState {
        uuid,
        rootino: NULL64,
        rbmino: NULL64,
        rsumino: NULL64,
        icount: 0,
        ifree: 0,
        fdblocks: g.initial_fdblocks(),
        inprogress: true,
    };
    let finished = sb::SbState {
        rootino,
        rbmino: rootino + 1,
        rsumino: rootino + 2,
        icount: 64,
        ifree: 61,
        fdblocks,
        inprogress: false,
        ..secondary
    };

    // prepare_devices: zero the ends of the device, whole blocks only.
    let dev_blocks_end = dev.size() / bs * bs;
    dev.write_zeroes(0, WHACK_SIZE.min(dev_blocks_end)).await?;
    let tail = dev_blocks_end.saturating_sub(WHACK_SIZE) / bs * bs;
    dev.write_zeroes(tail, dev_blocks_end - tail).await?;

    // libxfs_log_clear: zero the log, then its first record.
    let log_at = g.agb_to_byte(g.logagno, g.log_agbno);
    dev.write_zeroes(log_at, g.logblocks * bs).await?;
    let logsectsize = if g.sectsize > 512 { g.lsectsize } else { 0 };
    let rec = log::first_record(&uuid, log::clear_sunit(g.logsunit(), logsectsize));
    let mut first = vec![0u8; bs as usize];
    first[..rec.len()].copy_from_slice(&rec);
    dev.write_at(log_at, &first).await?;

    // Every AG: headers and roots, and the root inode chunk in AG 0. The
    // primary is written unfinished; rewrite_secondary_superblocks puts the
    // root inode in the last superblock and the middle one.
    let middle = if g.agcount > 2 { Some((g.agcount - 1) / 2) } else { None };
    let (g_ref, plans_ref) = (&g, &plans);
    stream::iter(0..g.agcount)
        .map(|agno| async move {
            let plan = &plans_ref[agno as usize];
            let mut sbs = secondary;
            if agno == 0 {
                sbs = sb::SbState { inprogress: true, ..finished };
            } else if agno == g_ref.agcount - 1 || Some(agno) == middle {
                sbs.rootino = rootino;
            }
            let head = ag_head(g_ref, plan, &uuid, &sbs);
            dev.write_at(g_ref.agb_to_byte(agno, 0), &head).await?;
            Ok::<_, Error>(())
        })
        .buffer_unordered(AG_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;

    let chunk = plans[0].chunk.expect("AG 0 holds the root chunk");
    let inodes = root_chunk(&g, chunk, &uuid, time, &mut gen);
    dev.write_at(g.agb_to_byte(0, chunk), &inodes).await?;
    dev.flush().await?;

    // Mark the filesystem finished.
    let head = ag_head(&g, &plans[0], &uuid, &finished);
    let hb = (g.header_blocks() * bs) as usize;
    dev.write_at(0, &head[..hb]).await?;
    dev.flush().await?;

    Ok(Report { geometry: g, uuid, rootino, fdblocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo512m() -> Geometry {
        Geometry::compute(&Params::new(), 512 << 20, 512, 512).unwrap()
    }

    /// The layout of mkfs.xfs 6.15.0's 512 MiB default, from xfs_db.
    #[test]
    fn plans_match_mkfs_xfs() {
        let g = geo512m();
        let ag0 = plan_ag(&g, 0).unwrap();
        let rec = |s, c| AllocRec { start: s, count: c };
        assert_eq!(ag0.free, vec![rec(13, 3), rec(24, 32744)]);
        assert_eq!(ag0.agfl, vec![7, 8, 9, 10, 11, 12]);
        assert_eq!(ag0.chunk, Some(16));
        let owners: Vec<(u32, u32, i64)> =
            ag0.rmap.iter().map(|r| (r.start, r.count, r.owner as i64)).collect();
        assert_eq!(
            owners,
            vec![(0, 1, -3), (1, 2, -5), (3, 2, -6), (5, 1, -5), (6, 1, -8), (7, 6, -5), (16, 8, -7)]
        );
        let ag2 = plan_ag(&g, 2).unwrap();
        assert_eq!(ag2.free, vec![rec(16397, 16371)]);
        assert_eq!(ag2.agfl, (16391..16397).collect::<Vec<_>>());
        let owners: Vec<(u32, u32, i64)> =
            ag2.rmap.iter().map(|r| (r.start, r.count, r.owner as i64)).collect();
        assert_eq!(owners[5..], [(7, 16384, -4), (16391, 6, -5)]);
    }

    #[test]
    fn rmap_merges_contiguous_same_owner() {
        let mut v = vec![RmapRec { start: 4, count: 1, owner: owner::AG }];
        rmap_insert(&mut v, RmapRec { start: 5, count: 6, owner: owner::AG });
        assert_eq!(v, vec![RmapRec { start: 4, count: 7, owner: owner::AG }]);
    }
}
