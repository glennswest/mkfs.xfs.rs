//! What `mkfs.xfs` would choose: block, sector and inode sizes, allocation
//! group count and size, the internal log's size and place, and the fixed
//! positions of every per-AG header and btree root.
//!
//! Each calculation names the xfsprogs function it reproduces (xfsprogs
//! 6.15.0, `mkfs/xfs_mkfs.c` unless noted). Where `mkfs.xfs` would need a
//! calculation this crate does not have yet — the minimum log size, which
//! is the whole transaction-reservation table — the configuration is refused
//! with [`Error::Unsupported`] rather than approximated.

use crate::error::{Error, Result};

/// `XFS_MIN_BLOCKSIZE` for a CRC (v5) filesystem.
pub const MIN_CRC_BLOCKSIZE: u32 = 1024;
/// `XFS_MAX_BLOCKSIZE`.
pub const MAX_BLOCKSIZE: u32 = 65536;
/// Largest block size whose default log this crate can size exactly: above
/// it the minimum log size (not implemented) exceeds the 64 MiB realistic
/// floor, and `mkfs.xfs` uses the minimum instead.
pub const MAX_SUPPORTED_BLOCKSIZE: u32 = 16384;
/// `XFS_MIN_SECTORSIZE`.
pub const MIN_SECTORSIZE: u32 = 512;
/// `XFS_MAX_SECTORSIZE`.
pub const MAX_SECTORSIZE: u32 = 32768;
/// `XFS_DINODE_MIN_SIZE`.
pub const DINODE_MIN_SIZE: u32 = 256;
/// `XFS_DINODE_MAX_SIZE`.
pub const DINODE_MAX_SIZE: u32 = 2048;
/// `XFS_INODE_BIG_CLUSTER_SIZE`.
pub const INODE_BIG_CLUSTER_SIZE: u32 = 8192;
/// `XFS_INODES_PER_CHUNK`.
pub const INODES_PER_CHUNK: u32 = 64;
/// `XFS_MAX_LOG_BLOCKS`.
pub const MAX_LOG_BLOCKS: u64 = 1024 * 1024;
/// `XFS_MIN_LOG_BYTES`.
pub const MIN_LOG_BYTES: u64 = 10 * 1024 * 1024;
/// `XFS_MAX_LOG_BYTES`: kept under 2^31 by a small amount.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024 * 1024 - MIN_LOG_BYTES;
/// `XFS_AG_MIN_BYTES`: 16 MiB.
pub const AG_MIN_BYTES: u64 = 1 << 24;
/// `XFS_AG_MAX_BYTES`: 1 TiB.
pub const AG_MAX_BYTES: u64 = 1 << 40;
/// `NULLAGNUMBER - 1`.
pub const MAX_AGNUMBER: u64 = 0xffff_fffe;
/// `XFS_ALLOCBT_AGFL_RESERVE`.
const ALLOCBT_AGFL_RESERVE: u64 = 4;
/// `XFS_AGFL` header bytes on a v5 filesystem (`struct xfs_agfl`).
pub const AGFL_HEADER: u32 = 36;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const TIB: u64 = 1 << 40;

/// The v5 features `mkfs.xfs` turns on or off. [`Default`] is what
/// `mkfs.xfs` 6.15 enables with no `-m`/`-i` options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    /// Free inode btree (`-m finobt`).
    pub finobt: bool,
    /// Reverse mapping btree (`-m rmapbt`).
    pub rmapbt: bool,
    /// Reference count btree, for reflink (`-m reflink`).
    pub reflink: bool,
    /// Inode btree block counters in the AGI (`-m inobtcount`).
    pub inobtcount: bool,
    /// Timestamps past 2038 (`-m bigtime`).
    pub bigtime: bool,
    /// 64-bit extent counters (`-i nrext64`).
    pub nrext64: bool,
    /// Sparse inode chunks (`-i sparse`).
    pub sparse: bool,
}

impl Default for Features {
    fn default() -> Self {
        Features {
            finobt: true,
            rmapbt: true,
            reflink: true,
            inobtcount: true,
            bigtime: true,
            nrext64: true,
            sparse: true,
        }
    }
}

/// A point in time as XFS stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// Seconds since the Unix epoch.
    pub secs: i64,
    /// Nanoseconds within the second.
    pub nsecs: u32,
}

impl Timestamp {
    /// The current time.
    pub fn now() -> Self {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp { secs: d.as_secs() as i64, nsecs: d.subsec_nanos() }
    }
}

/// What the caller asks for — the `mkfs.xfs` command line. Everything left
/// `None` is chosen the way `mkfs.xfs` chooses it.
#[derive(Debug, Clone, Default)]
pub struct Params {
    /// Filesystem size in bytes (`-d size=`). Default: the whole device.
    pub size: Option<u64>,
    /// Block size (`-b size=`). Default 4096.
    pub block_size: Option<u32>,
    /// Sector size (`-s size=`). Default: the device's physical sector.
    pub sector_size: Option<u32>,
    /// Inode size (`-i size=`). Default 512.
    pub inode_size: Option<u32>,
    /// Allocation group count (`-d agcount=`).
    pub agcount: Option<u64>,
    /// Allocation group size in bytes (`-d agsize=`).
    pub agsize: Option<u64>,
    /// Maximum percentage of space for inodes (`-i maxpct=`).
    pub imaxpct: Option<u8>,
    /// Volume label, at most 12 bytes (`-L`).
    pub label: Option<String>,
    /// Filesystem UUID (`-m uuid=`). Default: random.
    pub uuid: Option<[u8; 16]>,
    /// Creation time stamped on the root directory and the realtime
    /// inodes. Default: now.
    pub time: Option<Timestamp>,
    /// Seed for inode generation numbers, which `mkfs.xfs` draws at random.
    /// Default: random.
    pub gen_seed: Option<u64>,
    /// Feature switches.
    pub features: Features,
}

impl Params {
    /// Defaults throughout, as plain `mkfs.xfs <device>` would.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the filesystem size in bytes.
    pub fn size(mut self, bytes: u64) -> Self {
        self.size = Some(bytes);
        self
    }

    /// Set the block size.
    pub fn block_size(mut self, bytes: u32) -> Self {
        self.block_size = Some(bytes);
        self
    }

    /// Set the sector size.
    pub fn sector_size(mut self, bytes: u32) -> Self {
        self.sector_size = Some(bytes);
        self
    }

    /// Set the inode size.
    pub fn inode_size(mut self, bytes: u32) -> Self {
        self.inode_size = Some(bytes);
        self
    }

    /// Set the allocation group count.
    pub fn agcount(mut self, n: u64) -> Self {
        self.agcount = Some(n);
        self
    }

    /// Set the volume label.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Pin the UUID.
    pub fn uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = Some(uuid);
        self
    }

    /// Pin the creation time.
    pub fn time(mut self, secs: i64, nsecs: u32) -> Self {
        self.time = Some(Timestamp { secs, nsecs });
        self
    }

    /// Pin the inode generation numbers.
    pub fn gen_seed(mut self, seed: u64) -> Self {
        self.gen_seed = Some(seed);
        self
    }

    /// Set the feature switches.
    pub fn features(mut self, features: Features) -> Self {
        self.features = features;
        self
    }
}

/// Every number the formatter needs, fixed before a byte is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Geometry {
    /// Block size in bytes and its log2.
    pub blocksize: u32,
    /// log2 of the block size.
    pub blocklog: u8,
    /// Sector size in bytes.
    pub sectsize: u32,
    /// log2 of the sector size.
    pub sectlog: u8,
    /// Log sector size (the data sector size, for an internal log).
    pub lsectsize: u32,
    /// log2 of the log sector size.
    pub lsectlog: u8,
    /// Inode size in bytes.
    pub inodesize: u32,
    /// log2 of the inode size.
    pub inodelog: u8,
    /// Inodes per block.
    pub inopblock: u32,
    /// log2 of inodes per block.
    pub inopblog: u8,
    /// log2 of the directory block size.
    pub dirblocklog: u8,
    /// Data blocks.
    pub dblocks: u64,
    /// Blocks per allocation group (all but perhaps the last).
    pub agsize: u64,
    /// Allocation groups.
    pub agcount: u64,
    /// log2 of `agsize`, rounded up: the shift in block and inode numbers.
    pub agblklog: u8,
    /// Maximum percentage of space for inodes.
    pub imaxpct: u8,
    /// Log blocks.
    pub logblocks: u64,
    /// The AG holding the internal log.
    pub logagno: u64,
    /// The log's first block within its AG.
    pub log_agbno: u64,
    /// Features.
    pub features: Features,
    /// Volume label bytes (zero padded).
    pub label: [u8; 12],
}

/// Round `n` up to a multiple of `m`.
fn roundup(n: u64, m: u64) -> u64 {
    n.div_ceil(m) * m
}

/// `log2_roundup` (libfrog): the smallest `l` with `1 << l >= i`.
pub fn log2_roundup(i: u64) -> u8 {
    let mut l = 0u8;
    while (1u64 << l) < i {
        l += 1;
    }
    l
}

fn pow2_log(v: u32) -> u8 {
    v.trailing_zeros() as u8
}

/// `XFS_AG_MIN_BLOCKS`.
pub fn ag_min_blocks(blocklog: u8) -> u64 {
    AG_MIN_BYTES >> blocklog
}

/// `XFS_AG_MAX_BLOCKS`.
pub fn ag_max_blocks(blocklog: u8) -> u64 {
    (AG_MAX_BYTES - 1) >> blocklog
}

/// `calc_default_ag_geometry` (libxfs/topology.c): AG size and count for a
/// device of `dblocks` blocks. `multidisk` is a striped device; this crate
/// does not probe stripes yet, so the formatter passes `false`.
pub fn default_ag_geometry(blocklog: u8, dblocks: u64, multidisk: bool) -> (u64, u64) {
    let blocks_of = |bytes: u64| bytes >> blocklog;
    let max = ag_max_blocks(blocklog);
    let blocks = 'done: {
        // Past 32 TiB always the maximum AG size.
        if dblocks >= blocks_of(32 * TIB) {
            break 'done max;
        }
        let shift = if !multidisk && dblocks >= blocks_of(4 * TIB) {
            break 'done max;
        } else if !multidisk && dblocks >= blocks_of(128 * MIB) {
            2 // XFS_NOMULTIDISK_AGLOG: 4 AGs
        } else {
            let mut s = 5; // XFS_MULTIDISK_AGLOG: 32 AGs
            if dblocks <= blocks_of(512 * GIB) {
                s -= 1;
            }
            if dblocks <= blocks_of(8 * GIB) {
                s -= 1;
            }
            if dblocks < blocks_of(128 * MIB) {
                s -= 1;
            }
            if dblocks < blocks_of(64 * MIB) {
                s -= 1;
            }
            if dblocks < blocks_of(32 * MIB) {
                s -= 1;
            }
            s
        };
        let mut b = dblocks >> shift;
        if dblocks & ((1u64 << shift) - 1) != 0 && b < max {
            b += 1;
        }
        b
    };
    (blocks, dblocks / blocks + u64::from(dblocks % blocks != 0))
}

impl Geometry {
    /// Work out the geometry `mkfs.xfs` would choose for `params` on a device
    /// of `device_size` bytes with the given logical and physical sectors.
    pub fn compute(
        params: &Params,
        device_size: u64,
        logical_sector: u32,
        physical_sector: u32,
    ) -> Result<Geometry> {
        let f = params.features;
        if f.inobtcount && !f.finobt {
            return Err(Error::InvalidParams(
                "inobtcount requires finobt (-m inobtcount=1 needs finobt=1)".into(),
            ));
        }

        // validate_blocksize
        let blocksize = params.block_size.unwrap_or(4096);
        if !blocksize.is_power_of_two() || !(512..=MAX_BLOCKSIZE).contains(&blocksize) {
            return Err(Error::InvalidParams(format!("illegal block size {blocksize}")));
        }
        if blocksize < MIN_CRC_BLOCKSIZE {
            return Err(Error::InvalidParams(format!(
                "minimum block size for CRC enabled filesystems is {MIN_CRC_BLOCKSIZE} bytes"
            )));
        }
        if blocksize > MAX_SUPPORTED_BLOCKSIZE {
            return Err(Error::Unsupported(format!(
                "block size {blocksize}: mkfs.xfs sizes its log from the minimum log size \
                 there, which this crate does not calculate yet"
            )));
        }
        let blocklog = pow2_log(blocksize);

        // validate_sectorsize
        let logical = logical_sector.max(MIN_SECTORSIZE);
        let sectsize = match params.sector_size {
            Some(s) => s,
            None => {
                let mut physical = physical_sector.max(logical);
                if physical > MAX_SECTORSIZE {
                    physical = logical;
                }
                if blocksize < physical && blocksize >= logical {
                    logical
                } else {
                    physical
                }
            }
        };
        if !sectsize.is_power_of_two() || !(MIN_SECTORSIZE..=MAX_SECTORSIZE).contains(&sectsize) {
            return Err(Error::InvalidParams(format!("illegal sector size {sectsize}")));
        }
        if blocksize < sectsize {
            return Err(Error::InvalidParams(format!(
                "block size {blocksize} cannot be smaller than sector size {sectsize}"
            )));
        }
        if sectsize < logical {
            return Err(Error::InvalidParams(format!(
                "illegal sector size {sectsize}; hw sector is {logical}"
            )));
        }
        let sectlog = pow2_log(sectsize);

        // validate_dirblocksize: the default only.
        let dirblocklog = if blocksize < 4096 { 12 } else { blocklog };

        // validate_inodesize
        let inodesize = params.inode_size.unwrap_or(512);
        if !inodesize.is_power_of_two()
            || inodesize < 512
            || inodesize > DINODE_MAX_SIZE
            || inodesize > blocksize / 2
        {
            return Err(Error::InvalidParams(format!(
                "illegal inode size {inodesize} (512 to {} with {blocksize}-byte blocks)",
                (blocksize / 2).min(DINODE_MAX_SIZE)
            )));
        }
        let inodelog = pow2_log(inodesize);

        // validate_datadev
        let device_blocks = device_size >> blocklog;
        let mut dblocks = match params.size {
            Some(bytes) => {
                let b = bytes >> blocklog;
                if b > device_blocks {
                    return Err(Error::InvalidParams(format!(
                        "size {bytes} is too large, maximum is {device_blocks} blocks"
                    )));
                }
                b
            }
            None => device_blocks,
        };
        if dblocks < ag_min_blocks(blocklog) {
            return Err(Error::InvalidParams(format!(
                "size {dblocks} of data subvolume is too small, minimum {} blocks",
                ag_min_blocks(blocklog)
            )));
        }

        // calculate_initial_ag_geometry. The concurrency path (taken for a
        // non-rotational device) is not reproduced: this is the geometry
        // mkfs.xfs gives a file or a rotational disk.
        let (agsize, mut agcount) = if let Some(bytes) = params.agsize {
            if bytes % u64::from(blocksize) != 0 {
                return Err(Error::InvalidParams(format!(
                    "agsize ({bytes}) not a multiple of fs blk size ({blocksize})"
                )));
            }
            let s = bytes / u64::from(blocksize);
            if s == 0 {
                return Err(Error::InvalidParams("agsize 0".into()));
            }
            (s, dblocks.div_ceil(s))
        } else if let Some(n) = params.agcount {
            if n == 0 {
                return Err(Error::InvalidParams("agcount 0".into()));
            }
            (dblocks.div_ceil(n), n)
        } else {
            default_ag_geometry(blocklog, dblocks, false)
        };

        // align_ag_geometry, with no stripe unit: drop a last AG too small
        // to be one.
        if dblocks % agsize != 0 && dblocks % agsize < ag_min_blocks(blocklog) {
            if params.agcount.is_some() {
                return Err(Error::InvalidParams(format!(
                    "last AG size {} blocks too small, minimum size is {} blocks",
                    dblocks % agsize,
                    ag_min_blocks(blocklog)
                )));
            }
            dblocks = (agcount - 1) * agsize;
            agcount -= 1;
        }

        // validate_ag_geometry
        if agsize < ag_min_blocks(blocklog) {
            return Err(Error::InvalidParams(format!(
                "agsize ({agsize} blocks) too small, need at least {} blocks",
                ag_min_blocks(blocklog)
            )));
        }
        if agsize > ag_max_blocks(blocklog) {
            return Err(Error::InvalidParams(format!(
                "agsize ({agsize} blocks) too big, maximum is {} blocks",
                ag_max_blocks(blocklog)
            )));
        }
        if agsize > dblocks {
            return Err(Error::InvalidParams(format!(
                "agsize ({agsize} blocks) too big, data area is {dblocks} blocks"
            )));
        }
        if agcount > MAX_AGNUMBER {
            return Err(Error::InvalidParams(format!("{agcount} allocation groups is too many")));
        }

        // calculate_imaxpct
        let imaxpct = match params.imaxpct {
            Some(p) if p > 100 => {
                return Err(Error::InvalidParams(format!("imaxpct {p} is over 100")))
            }
            Some(p) => p,
            None if dblocks < TIB >> blocklog => 25,
            None if dblocks < (50 * TIB) >> blocklog => 5,
            None => 1,
        };

        let label = match &params.label {
            Some(l) if l.len() > 12 => {
                return Err(Error::InvalidParams(format!("label {l:?} is longer than 12 bytes")))
            }
            Some(l) => {
                let mut b = [0u8; 12];
                b[..l.len()].copy_from_slice(l.as_bytes());
                b
            }
            None => [0u8; 12],
        };

        let mut g = Geometry {
            blocksize,
            blocklog,
            sectsize,
            sectlog,
            lsectsize: sectsize,
            lsectlog: sectlog,
            inodesize,
            inodelog,
            inopblock: blocksize / inodesize,
            inopblog: blocklog - inodelog,
            dirblocklog,
            dblocks,
            agsize,
            agcount,
            agblklog: log2_roundup(agsize),
            imaxpct,
            logblocks: 0,
            logagno: 0,
            log_agbno: 0,
            features: f,
            label,
        };
        g.calculate_log_size()?;

        // validate_supported: what mkfs.xfs refuses without --unsupported.
        if g.dblocks < (300 * MIB) >> blocklog {
            return Err(Error::InvalidParams(
                "filesystem must be larger than 300MB (mkfs.xfs refuses smaller ones too)".into(),
            ));
        }
        if g.logblocks < (64 * MIB) >> blocklog {
            return Err(Error::InvalidParams("log size must be at least 64MB".into()));
        }
        if g.agcount < 2 {
            return Err(Error::InvalidParams(
                "filesystem must have at least 2 superblocks for redundancy".into(),
            ));
        }
        Ok(g)
    }

    /// `calculate_log_size`, for an internal log with no size given.
    fn calculate_log_size(&mut self) -> Result<()> {
        let blocklog = self.blocklog;
        // The minimum log size needs the transaction reservation table,
        // which this crate does not have. It only decides the size for a
        // filesystem under 300 MB, refused below, or where it exceeds the
        // 64 MiB realistic floor — block sizes over 16 KiB, refused above.
        if self.dblocks < (300 * MIB) >> blocklog {
            return Err(Error::Unsupported(
                "filesystems under 300 MB, whose log is the minimum log size".into(),
            ));
        }

        // "Make sure the log fits wholly within an AG", less one block.
        let max_logblocks = self.ag_max_usable() - 1;

        // A 2048:1 filesystem-to-log ratio...
        let mut logblocks = ((self.dblocks << blocklog) / 2048) >> blocklog;
        // ...but not below a realistic size...
        logblocks = logblocks.max((64 * MIB) >> blocklog);
        // ...and within the AG and the format's limits.
        logblocks = logblocks.min(max_logblocks);
        logblocks = logblocks.min(MAX_LOG_BLOCKS);
        if logblocks << blocklog > MAX_LOG_BYTES {
            logblocks = MAX_LOG_BYTES >> blocklog;
        }
        if logblocks > self.agsize - self.prealloc_blocks() {
            return Err(Error::InvalidParams(format!(
                "internal log size {logblocks} too large, must fit in allocation group"
            )));
        }

        self.logagno = self.agcount / 2;
        if self.logagno == 0 {
            // adjust_ag0_internal_logblocks: leave room for the root inode
            // chunk and an aligned freelist fixup.
            let ichunk = u64::from(INODES_PER_CHUNK * self.inodesize) >> blocklog;
            logblocks = logblocks.min(max_logblocks - ichunk * 4);
        }
        self.logblocks = logblocks;
        self.log_agbno = self.prealloc_blocks();
        Ok(())
    }

    /// Bytes per filesystem block, as a `u64`.
    pub fn bs(&self) -> u64 {
        u64::from(self.blocksize)
    }

    /// Blocks in allocation group `agno` — the last may be short.
    pub fn ag_blocks(&self, agno: u64) -> u64 {
        if agno == self.agcount - 1 {
            self.dblocks - agno * self.agsize
        } else {
            self.agsize
        }
    }

    /// `XFS_AGFL_BLOCK`: blocks taken by the four header sectors.
    pub fn header_blocks(&self) -> u64 {
        (4 * u64::from(self.sectsize)).div_ceil(self.bs())
    }

    /// `XFS_BNO_BLOCK`.
    pub fn bno_block(&self) -> u64 {
        self.header_blocks()
    }

    /// `XFS_CNT_BLOCK`.
    pub fn cnt_block(&self) -> u64 {
        self.bno_block() + 1
    }

    /// `XFS_IBT_BLOCK`.
    pub fn ibt_block(&self) -> u64 {
        self.cnt_block() + 1
    }

    /// `XFS_FIBT_BLOCK`.
    pub fn fibt_block(&self) -> u64 {
        self.ibt_block() + 1
    }

    /// `XFS_RMAP_BLOCK`.
    pub fn rmap_block(&self) -> u64 {
        if self.features.finobt {
            self.fibt_block() + 1
        } else {
            self.ibt_block() + 1
        }
    }

    /// `xfs_refc_block`.
    pub fn refc_block(&self) -> u64 {
        if self.features.rmapbt {
            self.rmap_block() + 1
        } else if self.features.finobt {
            self.fibt_block() + 1
        } else {
            self.ibt_block() + 1
        }
    }

    /// `xfs_prealloc_blocks`: the headers and btree roots at the start of
    /// every AG.
    pub fn prealloc_blocks(&self) -> u64 {
        if self.features.reflink {
            self.refc_block() + 1
        } else if self.features.rmapbt {
            self.rmap_block() + 1
        } else if self.features.finobt {
            self.fibt_block() + 1
        } else {
            self.ibt_block() + 1
        }
    }

    /// `xfs_alloc_ag_max_usable`.
    pub fn ag_max_usable(&self) -> u64 {
        let f = self.features;
        let blocks = self.header_blocks()
            + ALLOCBT_AGFL_RESERVE
            + 3
            + u64::from(f.finobt)
            + u64::from(f.rmapbt)
            + u64::from(f.reflink);
        self.agsize - blocks
    }

    /// `xfs_agfl_size`: free list slots in the AGFL sector.
    pub fn agfl_size(&self) -> u32 {
        (self.sectsize - AGFL_HEADER) / 4
    }

    /// `xfs_alloc_min_freelist` for an AG whose btrees are all one level:
    /// what `mkfs.xfs` fills each AGFL with.
    pub fn min_freelist(&self) -> u64 {
        // (level + 1) * 2 - 2 for the bno, cnt and rmap btrees. The
        // maximum heights that clamp this are never below 2.
        2 + 2 + if self.features.rmapbt { 2 } else { 0 }
    }

    /// Blocks in an inode chunk.
    pub fn chunk_blocks(&self) -> u64 {
        (u64::from(INODES_PER_CHUNK) * u64::from(self.inodesize)) >> self.blocklog
    }

    /// `sb_inoalignmt`.
    pub fn inoalignmt(&self) -> u64 {
        if self.features.sparse {
            self.chunk_blocks()
        } else {
            self.cluster_align()
        }
    }

    /// Inode cluster alignment in blocks: `XFS_INODE_BIG_CLUSTER_SIZE`
    /// scaled by inode size (v5), and `sb_spino_align` when sparse.
    pub fn cluster_align(&self) -> u64 {
        let cluster = INODE_BIG_CLUSTER_SIZE * (self.inodesize / DINODE_MIN_SIZE);
        u64::from(cluster >> self.blocklog)
    }

    /// `sb_spino_align`.
    pub fn spino_align(&self) -> u32 {
        if self.features.sparse {
            self.cluster_align() as u32
        } else {
            0
        }
    }

    /// Filesystem block number as XFS encodes it: the AG number above
    /// `agblklog` bits.
    pub fn agb_to_fsb(&self, agno: u64, agbno: u64) -> u64 {
        (agno << self.agblklog) | agbno
    }

    /// Byte offset of block `agbno` in AG `agno`.
    pub fn agb_to_byte(&self, agno: u64, agbno: u64) -> u64 {
        (agno * self.agsize + agbno) << self.blocklog
    }

    /// Disk address (512-byte units) of block `agbno` in AG `agno`.
    pub fn agb_to_daddr(&self, agno: u64, agbno: u64) -> u64 {
        (agno * self.agsize + agbno) << (self.blocklog - 9)
    }

    /// Inode number of the inode at `agbno`, slot `slot`, in AG `agno`.
    pub fn ino(&self, agno: u64, agbno: u64, slot: u64) -> u64 {
        let agino = (agbno << self.inopblog) | slot;
        (agno << (self.agblklog + self.inopblog)) | agino
    }

    /// `sb_logstart`.
    pub fn logstart(&self) -> u64 {
        self.agb_to_fsb(self.logagno, self.log_agbno)
    }

    /// `xfs_ialloc_calc_rootino`: where the root inode chunk goes in AG 0.
    pub fn root_chunk_agbno(&self) -> u64 {
        let f = self.features;
        let mut first = self.header_blocks() + 2 + 1 + self.min_freelist();
        first += u64::from(f.finobt) + u64::from(f.rmapbt) + u64::from(f.reflink);
        if self.logagno == 0 {
            first += self.logblocks;
        }
        let align = self.inoalignmt();
        if align > 1 {
            first = roundup(first, align);
        }
        first
    }

    /// The root directory's inode number.
    pub fn rootino(&self) -> u64 {
        self.ino(0, self.root_chunk_agbno(), 0)
    }

    /// Free blocks before the root inode chunk is allocated:
    /// `dblocks - agcount * prealloc - logblocks`. What the secondary
    /// superblocks record.
    pub fn initial_fdblocks(&self) -> u64 {
        self.dblocks - self.agcount * self.prealloc_blocks() - self.logblocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;
    const TIB: u64 = 1 << 40;

    fn geo(size: u64) -> Geometry {
        Geometry::compute(&Params::new(), size, 512, 512).unwrap()
    }

    /// `mkfs.xfs -N` 6.15.0 on sparse files of these sizes.
    #[test]
    fn matches_mkfs_xfs_dry_runs() {
        // (size, agcount, agsize, dblocks, logblocks, imaxpct)
        let table: &[(u64, u64, u64, u64, u64, u8)] = &[
            (300 << 20, 4, 19200, 76800, 16384, 25),
            (512 << 20, 4, 32768, 131072, 16384, 25),
            (GIB, 4, 65536, 262144, 16384, 25),
            (4 * GIB, 4, 262144, 1048576, 16384, 25),
            (16 * GIB, 4, 1048576, 4194304, 16384, 25),
            (100 * GIB, 4, 6553600, 26214400, 16384, 25),
            (TIB, 4, 67108864, 268435456, 131072, 5),
            (16 * TIB, 16, 268435455, 4294967280, 521728, 5),
            (1024 * TIB, 1024, 268435455, 274877905920, 521728, 1),
        ];
        for &(size, agcount, agsize, dblocks, logblocks, imaxpct) in table {
            let g = geo(size);
            assert_eq!(
                (g.agcount, g.agsize, g.dblocks, g.logblocks, g.imaxpct),
                (agcount, agsize, dblocks, logblocks, imaxpct),
                "size {size}"
            );
        }
    }

    #[test]
    fn default_layout_positions() {
        let g = geo(512 << 20);
        assert_eq!(g.prealloc_blocks(), 7);
        assert_eq!(g.logagno, 2);
        assert_eq!(g.logstart(), 65543);
        assert_eq!(g.rootino(), 128);
        assert_eq!(g.initial_fdblocks(), 114660);
        assert_eq!(g.agfl_size(), 119);
        assert_eq!((g.inoalignmt(), g.spino_align()), (8, 4));
    }

    #[test]
    fn refuses_what_it_cannot_reproduce() {
        assert!(matches!(
            Geometry::compute(&Params::new(), 64 << 20, 512, 512),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            Geometry::compute(&Params::new().block_size(65536), GIB, 512, 512),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn sector_size_follows_the_physical_sector() {
        let g = Geometry::compute(&Params::new(), GIB, 512, 4096).unwrap();
        assert_eq!(g.sectsize, 4096);
        // A 1 KiB block on a 512e drive falls back to the logical sector.
        let g = Geometry::compute(&Params::new().block_size(1024), GIB, 512, 4096).unwrap();
        assert_eq!(g.sectsize, 512);
    }
}
