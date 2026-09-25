//! Read an XFS filesystem's metadata into a flat, ordered list of named
//! fields — the form [`crate::compare`] diffs, and a readable dump.
//!
//! Every structure `mkfs.xfs` writes is covered: all superblocks, every
//! AGF, AGI and AGFL, every block of every per-AG btree (walked from its
//! root), every inode in every allocated chunk, and the log. Each field is
//! classed:
//!
//! - [`Class::Identity`] — the UUID, which a comparison pins or ignores;
//! - [`Class::Incidental`] — values `mkfs.xfs` draws at random or from the
//!   clock (timestamps, inode generations) and the raw CRCs that follow
//!   from them. Whether each CRC *verifies* is structural;
//! - [`Class::Structural`] — everything else. Any difference here is a
//!   difference in the filesystem.

use std::fmt::Write as _;

use crate::bytes::*;
use crate::crc;
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::structs::ag::{agf, agfl, agi, AGFL_MAGIC, AGF_MAGIC, AGI_MAGIC};
use crate::structs::btree::{self, magic};
use crate::structs::inode::{self, CORE_SIZE};
use crate::structs::{log, sb, NULL32};

/// How a field's difference is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A real difference in the filesystem.
    Structural,
    /// The filesystem UUID.
    Identity,
    /// Random or clock-derived, and the checksums that cover them.
    Incidental,
}

/// One named value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Where it is, e.g. `ag[2].agf.freeblks`.
    pub key: String,
    /// Its value, rendered.
    pub value: String,
    /// How a difference in it is judged.
    pub class: Class,
}

/// Everything read, in on-disk order.
#[derive(Debug, Clone, Default)]
pub struct Dump {
    /// The fields.
    pub fields: Vec<Field>,
}

impl Dump {
    fn push(&mut self, key: String, value: impl ToString, class: Class) {
        self.fields.push(Field { key, value: value.to_string(), class });
    }

    fn s(&mut self, key: String, value: impl ToString) {
        self.push(key, value, Class::Structural);
    }

    /// Look a field up by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|f| f.key == key).map(|f| f.value.as_str())
    }

    /// Render one field per line, `key = value`.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for f in &self.fields {
            let _ = writeln!(out, "{} = {}", f.key, f.value);
        }
        out
    }
}

fn uuid_str(b: &[u8]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn hex_trimmed(b: &[u8]) -> String {
    let end = b.iter().rposition(|&x| x != 0).map_or(0, |i| i + 1);
    b[..end].iter().map(|x| format!("{x:02x}")).collect()
}

/// The superblock fields a reader needs to find everything else.
#[derive(Debug, Clone)]
pub struct Geo {
    /// Block size.
    pub blocksize: u64,
    /// log2 block size.
    pub blocklog: u32,
    /// Sector size.
    pub sectsize: u64,
    /// Blocks per AG.
    pub agblocks: u64,
    /// AG count.
    pub agcount: u64,
    /// Data blocks.
    pub dblocks: u64,
    /// log2 of agblocks, rounded up.
    pub agblklog: u32,
    /// Inode size.
    pub inodesize: u64,
    /// log2 inodes per block.
    pub inopblog: u32,
    /// Log start (encoded fsb) and length.
    pub logstart: u64,
    #[allow(missing_docs)]
    pub logblocks: u64,
    /// Log sector size (0 for 512).
    pub logsectsize: u64,
    /// ro-compat and incompat features.
    pub ro_compat: u32,
    #[allow(missing_docs)]
    pub incompat: u32,
}

impl Geo {
    fn parse(b: &[u8]) -> Result<Geo> {
        use sb::off::*;
        if be32(b, MAGICNUM) != sb::MAGIC {
            return Err(Error::NotXfs("no XFSB magic at byte 0".into()));
        }
        if be16(b, VERSIONNUM) & 0xf != 5 {
            return Err(Error::NotXfs("not a v5 (CRC) superblock".into()));
        }
        let g = Geo {
            blocksize: u64::from(be32(b, BLOCKSIZE)),
            blocklog: u32::from(b[BLOCKLOG]),
            sectsize: u64::from(be16(b, SECTSIZE)),
            agblocks: u64::from(be32(b, AGBLOCKS)),
            agcount: u64::from(be32(b, AGCOUNT)),
            dblocks: be64(b, DBLOCKS),
            agblklog: u32::from(b[AGBLKLOG]),
            inodesize: u64::from(be16(b, INODESIZE)),
            inopblog: u32::from(b[INOPBLOG]),
            logstart: be64(b, LOGSTART),
            logblocks: u64::from(be32(b, LOGBLOCKS)),
            logsectsize: u64::from(be16(b, LOGSECTSIZE)),
            ro_compat: be32(b, FEATURES_RO_COMPAT),
            incompat: be32(b, FEATURES_INCOMPAT),
        };
        let ok = g.blocksize.is_power_of_two()
            && (512..=65536).contains(&g.blocksize)
            && 1u64 << g.blocklog == g.blocksize
            && g.sectsize.is_power_of_two()
            && (512..=32768).contains(&g.sectsize)
            && g.sectsize <= g.blocksize
            && g.inodesize.is_power_of_two()
            && (256..=2048).contains(&g.inodesize)
            && g.agcount > 0
            && g.agblocks > 0
            && g.agblocks <= 1u64 << g.agblklog
            && g.agblklog < 32
            && g.dblocks <= g.agcount * g.agblocks;
        if !ok {
            return Err(Error::Corrupt("superblock geometry".into()));
        }
        Ok(g)
    }

    fn byte(&self, agno: u64, agbno: u64) -> u64 {
        (agno * self.agblocks + agbno) << self.blocklog
    }

    fn ag_len(&self, agno: u64) -> u64 {
        if agno == self.agcount - 1 {
            self.dblocks - agno * self.agblocks
        } else {
            self.agblocks
        }
    }

    fn has(&self, ro: u32) -> bool {
        self.ro_compat & ro != 0
    }
}

async fn read<D: BlockDevice + ?Sized>(dev: &D, offset: u64, len: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    dev.read_at(offset, &mut buf).await?;
    Ok(buf)
}

/// Read the primary superblock's geometry.
pub async fn geometry<D: BlockDevice + ?Sized>(dev: &D) -> Result<Geo> {
    // The one read that is not a whole block: the block size is not known
    // yet. It is whole sectors of the device.
    let first = u64::from(dev.logical_sector_size().max(512));
    let head = read(dev, 0, first.min(dev.size())).await?;
    Geo::parse(&head)
}

fn dump_sb(d: &mut Dump, p: &str, b: &[u8], sectsize: usize) {
    use sb::off::*;
    let s = |d: &mut Dump, name: &str, v: u64| d.s(format!("{p}.{name}"), v);
    s(d, "magicnum", be32(b, MAGICNUM).into());
    s(d, "blocksize", be32(b, BLOCKSIZE).into());
    s(d, "dblocks", be64(b, DBLOCKS));
    s(d, "rblocks", be64(b, RBLOCKS));
    s(d, "rextents", be64(b, REXTENTS));
    d.push(format!("{p}.uuid"), uuid_str(&b[UUID..UUID + 16]), Class::Identity);
    s(d, "logstart", be64(b, LOGSTART));
    s(d, "rootino", be64(b, ROOTINO));
    s(d, "rbmino", be64(b, RBMINO));
    s(d, "rsumino", be64(b, RSUMINO));
    s(d, "rextsize", be32(b, REXTSIZE).into());
    s(d, "agblocks", be32(b, AGBLOCKS).into());
    s(d, "agcount", be32(b, AGCOUNT).into());
    s(d, "rbmblocks", be32(b, RBMBLOCKS).into());
    s(d, "logblocks", be32(b, LOGBLOCKS).into());
    d.s(format!("{p}.versionnum"), format!("{:#x}", be16(b, VERSIONNUM)));
    s(d, "sectsize", be16(b, SECTSIZE).into());
    s(d, "inodesize", be16(b, INODESIZE).into());
    s(d, "inopblock", be16(b, INOPBLOCK).into());
    d.s(format!("{p}.fname"), hex_trimmed(&b[FNAME..FNAME + 12]));
    for (name, o) in [
        ("blocklog", BLOCKLOG),
        ("sectlog", SECTLOG),
        ("inodelog", INODELOG),
        ("inopblog", INOPBLOG),
        ("agblklog", AGBLKLOG),
        ("rextslog", REXTSLOG),
        ("inprogress", INPROGRESS),
        ("imax_pct", IMAX_PCT),
    ] {
        s(d, name, b[o].into());
    }
    s(d, "icount", be64(b, ICOUNT));
    s(d, "ifree", be64(b, IFREE));
    s(d, "fdblocks", be64(b, FDBLOCKS));
    s(d, "frextents", be64(b, FREXTENTS));
    s(d, "uquotino", be64(b, UQUOTINO));
    s(d, "gquotino", be64(b, GQUOTINO));
    s(d, "qflags", be16(b, QFLAGS).into());
    s(d, "flags", b[FLAGS].into());
    s(d, "shared_vn", b[SHARED_VN].into());
    s(d, "inoalignmt", be32(b, INOALIGNMT).into());
    s(d, "unit", be32(b, UNIT).into());
    s(d, "width", be32(b, WIDTH).into());
    s(d, "dirblklog", b[DIRBLKLOG].into());
    s(d, "logsectlog", b[LOGSECTLOG].into());
    s(d, "logsectsize", be16(b, LOGSECTSIZE).into());
    s(d, "logsunit", be32(b, LOGSUNIT).into());
    for (name, o) in [
        ("features2", FEATURES2),
        ("bad_features2", BAD_FEATURES2),
        ("features_compat", FEATURES_COMPAT),
        ("features_ro_compat", FEATURES_RO_COMPAT),
        ("features_incompat", FEATURES_INCOMPAT),
        ("features_log_incompat", FEATURES_LOG_INCOMPAT),
    ] {
        d.s(format!("{p}.{name}"), format!("{:#x}", be32(b, o)));
    }
    s(d, "spino_align", be32(b, SPINO_ALIGN).into());
    s(d, "pquotino", be64(b, PQUOTINO));
    s(d, "lsn", be64(b, LSN));
    d.s(format!("{p}.meta_uuid"), hex_trimmed(&b[META_UUID..META_UUID + 16]));
    d.s(format!("{p}.tail"), hex_trimmed(&b[END..sectsize]));
    crc_fields(d, p, &b[..sectsize], CRC);
}

fn crc_fields(d: &mut Dump, p: &str, b: &[u8], off: usize) {
    d.push(format!("{p}.crc"), format!("{:#010x}", le32(b, off)), Class::Incidental);
    d.s(format!("{p}.crc_ok"), crc::verify(b, off));
}

fn dump_agf(d: &mut Dump, p: &str, b: &[u8]) {
    use agf::*;
    for (name, o) in [
        ("magicnum", MAGICNUM),
        ("versionnum", VERSIONNUM),
        ("seqno", SEQNO),
        ("length", LENGTH),
        ("bnoroot", BNO_ROOT),
        ("cntroot", CNT_ROOT),
        ("rmaproot", RMAP_ROOT),
        ("bnolevel", BNO_LEVEL),
        ("cntlevel", CNT_LEVEL),
        ("rmaplevel", RMAP_LEVEL),
        ("flfirst", FLFIRST),
        ("fllast", FLLAST),
        ("flcount", FLCOUNT),
        ("freeblks", FREEBLKS),
        ("longest", LONGEST),
        ("btreeblks", BTREEBLKS),
        ("rmapblocks", RMAP_BLOCKS),
        ("refcntblocks", REFCOUNT_BLOCKS),
        ("refcntroot", REFCOUNT_ROOT),
        ("refcntlevel", REFCOUNT_LEVEL),
    ] {
        d.s(format!("{p}.{name}"), be32(b, o));
    }
    d.push(format!("{p}.uuid"), uuid_str(&b[UUID..UUID + 16]), Class::Identity);
    d.s(format!("{p}.spare"), hex_trimmed(&b[REFCOUNT_LEVEL + 4..LSN]));
    d.s(format!("{p}.lsn"), be64(b, LSN));
    d.s(format!("{p}.tail"), hex_trimmed(&b[CRC + 4..]));
    crc_fields(d, p, b, CRC);
}

fn dump_agi(d: &mut Dump, p: &str, b: &[u8]) {
    use agi::*;
    for (name, o) in [
        ("magicnum", MAGICNUM),
        ("versionnum", VERSIONNUM),
        ("seqno", SEQNO),
        ("length", LENGTH),
        ("count", COUNT),
        ("root", ROOT),
        ("level", LEVEL),
        ("freecount", FREECOUNT),
        ("newino", NEWINO),
        ("dirino", DIRINO),
        ("free_root", FREE_ROOT),
        ("free_level", FREE_LEVEL),
        ("iblocks", IBLOCKS),
        ("fblocks", FBLOCKS),
    ] {
        d.s(format!("{p}.{name}"), be32(b, o));
    }
    let unlinked: Vec<String> = (0..64)
        .map(|i| be32(b, UNLINKED + 4 * i))
        .filter(|&v| v != NULL32)
        .map(|v| v.to_string())
        .collect();
    d.s(format!("{p}.unlinked"), unlinked.join(","));
    d.push(format!("{p}.uuid"), uuid_str(&b[UUID..UUID + 16]), Class::Identity);
    d.s(format!("{p}.lsn"), be64(b, LSN));
    d.s(format!("{p}.pad"), be32(b, CRC + 4));
    d.s(format!("{p}.tail"), hex_trimmed(&b[END..]));
    crc_fields(d, p, b, CRC);
}

fn dump_agfl(d: &mut Dump, p: &str, b: &[u8]) {
    use agfl::*;
    d.s(format!("{p}.magicnum"), be32(b, MAGICNUM));
    d.s(format!("{p}.seqno"), be32(b, SEQNO));
    d.push(format!("{p}.uuid"), uuid_str(&b[UUID..UUID + 16]), Class::Identity);
    d.s(format!("{p}.lsn"), be64(b, LSN));
    let list: Vec<String> = (0..(b.len() - BNO) / 4)
        .map(|i| (i, be32(b, BNO + 4 * i)))
        .filter(|&(_, v)| v != NULL32)
        .map(|(i, v)| format!("{i}:{v}"))
        .collect();
    d.s(format!("{p}.bno"), list.join(","));
    crc_fields(d, p, b, CRC);
}

/// A per-AG btree: its name in the dump, magic, and key and record sizes.
struct Tree {
    name: &'static str,
    magic: u32,
    key: usize,
    rec: usize,
}

const BNOBT: Tree = Tree { name: "bnobt", magic: magic::BNO, key: 8, rec: btree::ALLOC_REC };
const CNTBT: Tree = Tree { name: "cntbt", magic: magic::CNT, key: 8, rec: btree::ALLOC_REC };
const INOBT: Tree = Tree { name: "inobt", magic: magic::INO, key: 4, rec: btree::INOBT_REC };
const FINOBT: Tree = Tree { name: "finobt", magic: magic::FINO, key: 4, rec: btree::INOBT_REC };
// Overlapping btree: every node entry holds a low and a high key.
const RMAPBT: Tree = Tree { name: "rmapbt", magic: magic::RMAP, key: 40, rec: btree::RMAP_REC };
const REFCBT: Tree = Tree { name: "refcntbt", magic: magic::REFC, key: 4, rec: btree::REFC_REC };

/// Walk a btree from `root`, dumping every block. Returns the leaf records,
/// raw, in order.
async fn dump_tree<D: BlockDevice + ?Sized>(
    dev: &D,
    g: &Geo,
    d: &mut Dump,
    agno: u64,
    t: &Tree,
    root: u64,
) -> Result<Vec<Vec<u8>>> {
    use btree::off;
    let p = format!("ag[{agno}].{}", t.name);
    let mut level_blocks = vec![root];
    let mut leaves = Vec::new();
    let mut depth = 0;
    while !level_blocks.is_empty() {
        let mut next = Vec::new();
        for (i, &agbno) in level_blocks.iter().enumerate() {
            let bp = format!("{p}.l{depth}[{i}]");
            if agbno >= g.ag_len(agno) {
                d.s(format!("{bp}.block"), format!("{agbno} (outside the AG)"));
                continue;
            }
            let b = read(dev, g.byte(agno, agbno), g.blocksize).await?;
            d.s(format!("{bp}.block"), agbno);
            let m = be32(&b, off::MAGIC);
            d.s(format!("{bp}.magic"), format!("{m:#x}"));
            if m != t.magic {
                continue;
            }
            let level = be16(&b, off::LEVEL);
            let n = be16(&b, off::NUMRECS) as usize;
            d.s(format!("{bp}.level"), level);
            d.s(format!("{bp}.numrecs"), n);
            d.s(format!("{bp}.leftsib"), be32(&b, off::LEFTSIB));
            d.s(format!("{bp}.rightsib"), be32(&b, off::RIGHTSIB));
            d.s(format!("{bp}.blkno"), be64(&b, off::BLKNO));
            d.s(format!("{bp}.lsn"), be64(&b, off::LSN));
            d.push(format!("{bp}.uuid"), uuid_str(&b[off::UUID..off::UUID + 16]), Class::Identity);
            d.s(format!("{bp}.owner"), be32(&b, off::OWNER));
            crc_fields(d, &bp, &b, off::CRC);
            let space = b.len() - off::RECS;
            if level == 0 {
                let n = n.min(space / t.rec);
                for r in 0..n {
                    let at = off::RECS + r * t.rec;
                    let rec = b[at..at + t.rec].to_vec();
                    d.s(format!("{bp}.rec[{r}]"), hex_trimmed_full(&rec));
                    leaves.push(rec);
                }
                d.s(format!("{bp}.slack"), hex_trimmed(&b[off::RECS + n * t.rec..]));
            } else {
                let maxrecs = space / (t.key + 4);
                let n = n.min(maxrecs);
                for r in 0..n {
                    let k = off::RECS + r * t.key;
                    let ptr = be32(&b, off::RECS + maxrecs * t.key + 4 * r);
                    d.s(format!("{bp}.key[{r}]"), hex_trimmed_full(&b[k..k + t.key]));
                    d.s(format!("{bp}.ptr[{r}]"), ptr);
                    next.push(u64::from(ptr));
                }
            }
        }
        level_blocks = next;
        depth += 1;
        if depth > 16 {
            return Err(Error::Corrupt(format!("{p} is deeper than any real btree")));
        }
    }
    Ok(leaves)
}

fn hex_trimmed_full(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn dump_inode(d: &mut Dump, p: &str, b: &[u8]) {
    use inode::off::*;
    let flags2 = be64(b, FLAGS2);
    let bigtime = flags2 & inode::DIFLAG2_BIGTIME != 0;
    d.s(format!("{p}.magic"), format!("{:#x}", be16(b, MAGIC)));
    d.s(format!("{p}.mode"), format!("{:o}", be16(b, MODE)));
    d.s(format!("{p}.version"), b[VERSION]);
    d.s(format!("{p}.format"), b[FORMAT]);
    d.s(format!("{p}.metatype"), be16(b, METATYPE));
    d.s(format!("{p}.uid"), be32(b, UID));
    d.s(format!("{p}.gid"), be32(b, GID));
    d.s(format!("{p}.nlink"), be32(b, NLINK));
    d.s(format!("{p}.projid"), (u32::from(be16(b, PROJID_HI)) << 16) | u32::from(be16(b, PROJID_LO)));
    d.s(format!("{p}.big_nextents"), be64(b, BIG_NEXTENTS));
    for (name, o) in [("atime", ATIME), ("mtime", MTIME), ("ctime", CTIME), ("crtime", CRTIME)] {
        let t = inode::decode_time(be64(b, o), bigtime);
        d.push(format!("{p}.{name}"), format!("{}.{:09}", t.secs, t.nsecs), Class::Incidental);
    }
    d.s(format!("{p}.size"), be64(b, SIZE));
    d.s(format!("{p}.nblocks"), be64(b, NBLOCKS));
    d.s(format!("{p}.extsize"), be32(b, EXTSIZE));
    d.s(format!("{p}.nextents"), be32(b, NEXTENTS));
    d.s(format!("{p}.anextents"), be16(b, ANEXTENTS));
    d.s(format!("{p}.forkoff"), b[FORKOFF]);
    d.s(format!("{p}.aformat"), b[AFORMAT]);
    d.s(format!("{p}.dmevmask"), be32(b, DMEVMASK));
    d.s(format!("{p}.dmstate"), be16(b, DMSTATE));
    d.s(format!("{p}.flags"), format!("{:#x}", be16(b, FLAGS)));
    d.push(format!("{p}.gen"), be32(b, GEN), Class::Incidental);
    d.s(format!("{p}.next_unlinked"), be32(b, NEXT_UNLINKED));
    d.s(format!("{p}.changecount"), be64(b, CHANGECOUNT));
    d.s(format!("{p}.lsn"), be64(b, LSN));
    d.s(format!("{p}.flags2"), format!("{flags2:#x}"));
    d.s(format!("{p}.cowextsize"), be32(b, COWEXTSIZE));
    d.s(format!("{p}.pad2"), hex_trimmed(&b[COWEXTSIZE + 4..CRTIME]));
    d.s(format!("{p}.ino"), be64(b, INO));
    d.push(format!("{p}.uuid"), uuid_str(&b[UUID..UUID + 16]), Class::Identity);
    d.s(format!("{p}.literal"), hex_trimmed(&b[CORE_SIZE..]));
    crc_fields(d, p, b, CRC);
}

/// Dump every piece of metadata on `dev`.
pub async fn dump<D: BlockDevice + ?Sized>(dev: &D) -> Result<Dump> {
    let g = geometry(dev).await?;
    let mut d = Dump::default();
    let bs = g.blocksize;
    let ss = g.sectsize as usize;
    let hblocks = (4 * g.sectsize).div_ceil(bs);
    let has_finobt = g.has(sb::version::RO_FINOBT);
    let has_rmap = g.has(sb::version::RO_RMAPBT);
    let has_reflink = g.has(sb::version::RO_REFLINK);
    let sparse = g.incompat & sb::version::IN_SPINODES != 0;

    for agno in 0..g.agcount {
        let head = read(dev, g.byte(agno, 0), hblocks * bs).await?;
        let p = format!("ag[{agno}]");
        dump_sb(&mut d, &format!("{p}.sb"), &head[..ss], ss);
        let agf_b = &head[ss..2 * ss];
        let agi_b = &head[2 * ss..3 * ss];
        let agfl_b = &head[3 * ss..4 * ss];
        if be32(agf_b, agf::MAGICNUM) != AGF_MAGIC
            || be32(agi_b, agi::MAGICNUM) != AGI_MAGIC
            || be32(agfl_b, agfl::MAGICNUM) != AGFL_MAGIC
        {
            d.s(format!("{p}.headers"), "bad magic");
            continue;
        }
        dump_agf(&mut d, &format!("{p}.agf"), agf_b);
        dump_agi(&mut d, &format!("{p}.agi"), agi_b);
        dump_agfl(&mut d, &format!("{p}.agfl"), agfl_b);

        let root = |b: &[u8], o: usize| u64::from(be32(b, o));
        dump_tree(dev, &g, &mut d, agno, &BNOBT, root(agf_b, agf::BNO_ROOT)).await?;
        dump_tree(dev, &g, &mut d, agno, &CNTBT, root(agf_b, agf::CNT_ROOT)).await?;
        if has_rmap {
            dump_tree(dev, &g, &mut d, agno, &RMAPBT, root(agf_b, agf::RMAP_ROOT)).await?;
        }
        if has_reflink {
            dump_tree(dev, &g, &mut d, agno, &REFCBT, root(agf_b, agf::REFCOUNT_ROOT)).await?;
        }
        let chunks = dump_tree(dev, &g, &mut d, agno, &INOBT, root(agi_b, agi::ROOT)).await?;
        if has_finobt {
            dump_tree(dev, &g, &mut d, agno, &FINOBT, root(agi_b, agi::FREE_ROOT)).await?;
        }

        // Every inode of every chunk the inode btree lists, holes excepted.
        for rec in chunks {
            let startino = u64::from(be32(&rec, 0));
            let holemask = if sparse { be16(&rec, 4) } else { 0 };
            for i in 0..64u64 {
                if holemask & (1 << (i / 4)) != 0 {
                    continue;
                }
                let agino = startino + i;
                let agbno = agino >> g.inopblog;
                let slot = agino & ((1 << g.inopblog) - 1);
                if agbno >= g.ag_len(agno) {
                    return Err(Error::Corrupt(format!("inode chunk at {startino} runs out of AG {agno}")));
                }
                let block = read(dev, g.byte(agno, agbno), bs).await?;
                let at = (slot * g.inodesize) as usize;
                let ino = (agno << (g.agblklog + g.inopblog)) | agino;
                dump_inode(&mut d, &format!("inode[{ino}]"), &block[at..at + g.inodesize as usize]);
            }
        }
    }

    // The log: its first record, and whether everything after it is zero.
    let log_ag = g.logstart >> g.agblklog;
    let log_agbno = g.logstart & ((1 << g.agblklog) - 1);
    let log_at = g.byte(log_ag, log_agbno);
    let first = read(dev, log_at, bs).await?;
    let sunit = log::clear_sunit(g.logsectsize as u32);
    let rec_len = if sunit > 0 { (sunit as usize).div_ceil(512) } else { 2 }.max(2) * 512;
    {
        use log::off::*;
        let h = &first[..512];
        for (name, o) in [
            ("magicno", MAGICNO),
            ("cycle", CYCLE),
            ("version", VERSION),
            ("len", LEN),
            ("crc", CRC),
            ("prev_block", PREV_BLOCK),
            ("num_logops", NUM_LOGOPS),
            ("cycle_data0", CYCLE_DATA),
            ("fmt", FMT),
            ("size", SIZE),
        ] {
            d.s(format!("log.header.{name}"), format!("{:#x}", be32(h, o)));
        }
        d.s("log.header.lsn".into(), format!("{:#x}", be64(h, LSN)));
        d.s("log.header.tail_lsn".into(), format!("{:#x}", be64(h, TAIL_LSN)));
        d.push("log.header.uuid".into(), uuid_str(&h[FS_UUID..FS_UUID + 16]), Class::Identity);
        d.s("log.header.rest".into(), hex_trimmed(&h[SIZE + 4..]));
        d.s("log.record".into(), hex_trimmed(&first[512..rec_len.min(first.len())]));
    }
    let mut nonzero = u64::from(first[rec_len.min(first.len())..].iter().any(|&b| b != 0));
    let log_len = g.logblocks * bs;
    let mut at = bs;
    while at < log_len {
        let n = (1u64 << 20).min(log_len - at);
        let buf = read(dev, log_at + at, n).await?;
        nonzero += buf.chunks(bs as usize).filter(|c| c.iter().any(|&b| b != 0)).count() as u64;
        at += n;
    }
    d.s("log.nonzero_blocks_after_first_record".into(), nonzero);
    Ok(d)
}
