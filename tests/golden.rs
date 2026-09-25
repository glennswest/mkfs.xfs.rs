//! Our filesystems, diffed against real `mkfs.xfs` output.
//!
//! Each golden image was made by `mkfs.xfs` 6.15.0 with the UUID pinned
//! (`tests/golden/capture.py`; the exact command is in `NAME.args`). This
//! formats the same size with the same options and requires every
//! structural field to match. The UUID matches too; timestamps, inode
//! generation numbers and the CRCs over them are drawn at random by
//! `mkfs.xfs` and are the only differences allowed.

mod common;

use mkfs_xfs::compare::{compare, structural};
use mkfs_xfs::device::{BlockDevice, MemDevice};
use mkfs_xfs::format::format;
use mkfs_xfs::geometry::{Features, Params};
use mkfs_xfs::inspect::Class;

use common::{golden, GOLDEN_UUID};

/// The cases in `tests/golden/capture.py`, as `Params`.
fn cases() -> Vec<(&'static str, Params)> {
    let p = || Params::new().uuid(GOLDEN_UUID);
    let minimal = Features {
        finobt: false,
        rmapbt: false,
        reflink: false,
        inobtcount: false,
        bigtime: true,
        nrext64: false,
        sparse: false,
    };
    vec![
        ("xfs-512m-default", p()),
        ("xfs-300m-default", p()),
        ("xfs-1g-b1024", p().block_size(1024)),
        ("xfs-2g-b2048-i1024", p().block_size(2048).inode_size(1024)),
        ("xfs-2g-s4096", p().sector_size(4096)),
        ("xfs-1g-agcount7-label", p().agcount(7).label("golden")),
        ("xfs-600m-agcount2", p().agcount(2)),
        ("xfs-512m-v5-minimal", p().features(minimal)),
        ("xfs-1g-b16384", p().block_size(16384)),
    ]
}

#[tokio::test]
async fn every_golden_matches_mkfs_xfs() {
    let mut failed = Vec::new();
    for (name, params) in cases() {
        let theirs = golden(name).await;
        let ours = MemDevice::new(theirs.size());
        format(&ours, &params).await.unwrap_or_else(|e| panic!("{name}: format: {e}"));
        let diffs = compare(&ours, &theirs).await.unwrap_or_else(|e| panic!("{name}: compare: {e}"));
        let bad: Vec<_> = diffs.iter().filter(|d| d.class != Class::Incidental).collect();
        if bad.is_empty() {
            println!("{name}: identical but for {} incidental fields", diffs.len());
        } else {
            println!("{name}: {} differences that matter:", bad.len());
            for d in bad.iter().take(60) {
                println!("  {d}");
            }
            failed.push(name);
        }
        assert!(structural(&diffs).len() <= bad.len());
    }
    assert!(failed.is_empty(), "differ from mkfs.xfs: {failed:?}");
}

/// The comparison is only worth something if it notices a difference.
#[tokio::test]
async fn compare_notices_a_changed_field() {
    let theirs = golden("xfs-512m-default").await;
    let ours = MemDevice::new(theirs.size());
    format(&ours, &Params::new().uuid(GOLDEN_UUID).agcount(5)).await.unwrap();
    let diffs = compare(&ours, &theirs).await.unwrap();
    assert!(diffs.iter().any(|d| d.key == "ag[0].sb.agcount" && d.class == Class::Structural));
}
