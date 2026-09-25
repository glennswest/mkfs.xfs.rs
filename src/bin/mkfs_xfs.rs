//! `mkfs-xfs`: the command-line formatter, with `mkfs.xfs`'s option syntax
//! for the options this crate reproduces.

use anyhow::{bail, Context, Result};
use clap::Parser;

use mkfs_xfs::device::FileDevice;
use mkfs_xfs::format::format;
use mkfs_xfs::geometry::{Geometry, Params};

/// Make an XFS filesystem, as mkfs.xfs would.
#[derive(Parser, Debug)]
#[command(name = "mkfs-xfs", version, about)]
struct Cli {
    /// Block size: -b size=N
    #[arg(short = 'b', value_name = "size=N")]
    block: Vec<String>,
    /// Data section: -d size=N,agcount=N,agsize=N
    #[arg(short = 'd', value_name = "opts")]
    data: Vec<String>,
    /// Inodes: -i size=N,maxpct=N,sparse=0|1,nrext64=0|1
    #[arg(short = 'i', value_name = "opts")]
    inode: Vec<String>,
    /// Metadata: -m uuid=U,finobt=0|1,rmapbt=0|1,reflink=0|1,inobtcount=0|1,bigtime=0|1
    #[arg(short = 'm', value_name = "opts")]
    meta: Vec<String>,
    /// Sector size: -s size=N
    #[arg(short = 's', value_name = "size=N")]
    sector: Vec<String>,
    /// Volume label (at most 12 bytes).
    #[arg(short = 'L')]
    label: Option<String>,
    /// Print the geometry and write nothing.
    #[arg(short = 'N')]
    dry_run: bool,
    /// Quiet.
    #[arg(short = 'q')]
    quiet: bool,
    /// Accepted for compatibility: overwriting is not refused.
    #[arg(short = 'f')]
    force: bool,
    /// Create the file with this size (e.g. 1g) before formatting it.
    #[arg(long, value_name = "SIZE")]
    create: Option<String>,
    /// The device or image file.
    device: String,
}

/// A size with an optional binary suffix: 4096, 64k, 512m, 16g, 2t, 1p.
fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (num, shift) = match s.chars().last() {
        Some('k') => (&s[..s.len() - 1], 10),
        Some('m') => (&s[..s.len() - 1], 20),
        Some('g') => (&s[..s.len() - 1], 30),
        Some('t') => (&s[..s.len() - 1], 40),
        Some('p') => (&s[..s.len() - 1], 50),
        _ => (&s[..], 0),
    };
    let n: u64 = num.parse().with_context(|| format!("bad size {s:?}"))?;
    n.checked_mul(1 << shift).with_context(|| format!("size {s:?} overflows"))
}

fn parse_bool(s: &str) -> Result<bool> {
    match s {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => bail!("expected 0 or 1, got {s:?}"),
    }
}

fn parse_uuid(s: &str) -> Result<[u8; 16]> {
    Ok(*uuid::Uuid::parse_str(s).with_context(|| format!("bad uuid {s:?}"))?.as_bytes())
}

fn subopts(list: &[String]) -> Vec<(String, String)> {
    list.iter()
        .flat_map(|o| o.split(','))
        .filter(|kv| !kv.is_empty())
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (kv.to_string(), "1".to_string()),
        })
        .collect()
}

fn params(cli: &Cli) -> Result<Params> {
    let mut p = Params::new();
    for (k, v) in subopts(&cli.block) {
        match k.as_str() {
            "size" => p.block_size = Some(parse_size(&v)? as u32),
            _ => bail!("unknown -b option {k}"),
        }
    }
    for (k, v) in subopts(&cli.sector) {
        match k.as_str() {
            "size" => p.sector_size = Some(parse_size(&v)? as u32),
            _ => bail!("unknown -s option {k}"),
        }
    }
    for (k, v) in subopts(&cli.data) {
        match k.as_str() {
            "size" => p.size = Some(parse_size(&v)?),
            "agcount" => p.agcount = Some(v.parse()?),
            "agsize" => p.agsize = Some(parse_size(&v)?),
            _ => bail!("-d {k} is not supported"),
        }
    }
    for (k, v) in subopts(&cli.inode) {
        match k.as_str() {
            "size" => p.inode_size = Some(parse_size(&v)? as u32),
            "maxpct" => p.imaxpct = Some(v.parse()?),
            "sparse" => p.features.sparse = parse_bool(&v)?,
            "nrext64" => p.features.nrext64 = parse_bool(&v)?,
            _ => bail!("-i {k} is not supported"),
        }
    }
    for (k, v) in subopts(&cli.meta) {
        match k.as_str() {
            "uuid" => p.uuid = Some(parse_uuid(&v)?),
            "crc" if v == "1" => {}
            "finobt" => p.features.finobt = parse_bool(&v)?,
            "rmapbt" => p.features.rmapbt = parse_bool(&v)?,
            "reflink" => p.features.reflink = parse_bool(&v)?,
            "inobtcount" => p.features.inobtcount = parse_bool(&v)?,
            "bigtime" => p.features.bigtime = parse_bool(&v)?,
            _ => bail!("-m {k}={v} is not supported"),
        }
    }
    p.label = cli.label.clone();
    Ok(p)
}

fn report(g: &Geometry, name: &str) {
    let f = g.features;
    let b = |x: bool| u8::from(x);
    println!(
        "meta-data={name:<22} isize={:<6} agcount={}, agsize={} blks",
        g.inodesize, g.agcount, g.agsize
    );
    println!("         ={:<22} sectsz={:<5} attr=2, projid32bit=1", "", g.sectsize);
    println!(
        "         ={:<22} crc=1        finobt={}, sparse={}, rmapbt={}",
        "",
        b(f.finobt),
        b(f.sparse),
        b(f.rmapbt)
    );
    println!(
        "         ={:<22} reflink={}    bigtime={} inobtcount={} nrext64={}",
        "",
        b(f.reflink),
        b(f.bigtime),
        b(f.inobtcount),
        b(f.nrext64)
    );
    println!("data     ={:<22} bsize={:<6} blocks={}, imaxpct={}", "", g.blocksize, g.dblocks, g.imaxpct);
    println!(
        "naming   =version 2              bsize={:<6} ascii-ci=0, ftype=1, parent=0",
        1u32 << g.dirblocklog
    );
    println!("log      =internal log           bsize={:<6} blocks={}, version=2", g.blocksize, g.logblocks);
    println!("         ={:<22} sectsz={:<5} sunit=0 blks, lazy-count=1", "", g.lsectsize);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let _ = cli.force;
    let p = params(&cli)?;
    let dev = match &cli.create {
        Some(size) => FileDevice::create(&cli.device, parse_size(size)?).await?,
        None => FileDevice::open(&cli.device).await?,
    };
    if cli.dry_run {
        use mkfs_xfs::device::BlockDevice;
        let g = Geometry::compute(&p, dev.size(), dev.logical_sector_size(), dev.physical_sector_size())?;
        report(&g, &cli.device);
        return Ok(());
    }
    let r = format(&dev, &p).await?;
    if !cli.quiet {
        report(&r.geometry, &cli.device);
    }
    Ok(())
}
