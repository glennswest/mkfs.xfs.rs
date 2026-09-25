//! Against the `mkfs.xfs` and `xfs_repair` installed here, when there are
//! any (dev.g8.lo has xfsprogs; elsewhere these tests report a skip).
//!
//! For each case: real `mkfs.xfs` formats a sparse file, this crate formats
//! another of the same size, the two are compared field by field, and
//! `xfs_repair -n` must pass ours. Sizes go well past what the vendored
//! goldens hold — to 8 TiB, where the log reaches its 2 GiB ceiling
//! and AGs their 1 TiB one.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use mkfs_xfs::compare::compare;
use mkfs_xfs::device::FileDevice;
use mkfs_xfs::format::format;
use mkfs_xfs::geometry::Params;
use mkfs_xfs::inspect::Class;

use common::GOLDEN_UUID;

const UUID: &str = "12345678-1234-5678-9abc-123456789abc";

fn have(tool: &str) -> bool {
    Command::new(tool).arg("-V").output().map(|o| o.status.success()).unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("live");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// (name, size, mkfs.xfs options, the same as Params)
fn cases() -> Vec<(&'static str, u64, Vec<&'static str>, Params)> {
    const G: u64 = 1 << 30;
    let p = || Params::new().uuid(GOLDEN_UUID);
    vec![
        ("16g", 16 * G, vec![], p()),
        ("100g", 100 * G, vec![], p()),
        ("1t", 1 << 40, vec![], p()),
        ("5t", 5 << 40, vec![], p()),
        ("8t-b1024", 8 << 40, vec!["-b", "size=1024"], p().block_size(1024)),
        ("3g-agcount9-b8192", 3 * G, vec!["-d", "agcount=9", "-b", "size=8192"],
            p().agcount(9).block_size(8192)),
        ("1g-i2048", G, vec!["-i", "size=2048"], p().inode_size(2048)),
        ("777m-odd", 777 * (1 << 20) + 12345, vec![], p()),
    ]
}

#[tokio::test]
async fn ours_match_mkfs_xfs_and_pass_xfs_repair() {
    if !have("mkfs.xfs") {
        println!("SKIP: no mkfs.xfs on PATH");
        return;
    }
    let repair = have("xfs_repair");
    if !repair {
        println!("note: no xfs_repair on PATH; comparing only");
    }
    let mut failed = Vec::new();
    for (name, size, opts, params) in cases() {
        let theirs_path = scratch(&format!("{name}.theirs"));
        let ours_path = scratch(&format!("{name}.ours"));
        let _ = std::fs::remove_file(&theirs_path);
        let f = std::fs::File::create(&theirs_path).unwrap();
        f.set_len(size).unwrap();
        drop(f);
        let st = Command::new("mkfs.xfs")
            .args(["-q", "-f", "-m", &format!("uuid={UUID}")])
            .args(&opts)
            .arg(&theirs_path)
            .status()
            .unwrap();
        assert!(st.success(), "{name}: mkfs.xfs failed");

        let ours = FileDevice::create(&ours_path, size).await.unwrap();
        format(&ours, &params).await.unwrap_or_else(|e| panic!("{name}: format: {e}"));
        let theirs = FileDevice::open(&theirs_path).await.unwrap();
        let diffs = compare(&ours, &theirs).await.unwrap();
        let bad: Vec<_> = diffs.iter().filter(|d| d.class != Class::Incidental).collect();
        if !bad.is_empty() {
            println!("{name}: {} differences from mkfs.xfs:", bad.len());
            for d in bad.iter().take(40) {
                println!("  {d}");
            }
            failed.push(name.to_string());
        }
        drop(ours);

        if repair {
            let out = Command::new("xfs_repair").args(["-n", "-f"]).arg(&ours_path).output().unwrap();
            if !out.status.success() {
                println!("{name}: xfs_repair -n failed:\n{}{}",
                    String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                failed.push(format!("{name} (xfs_repair)"));
            } else {
                println!("{name}: matches mkfs.xfs, xfs_repair -n clean");
            }
        }
        let _ = std::fs::remove_file(&theirs_path);
        let _ = std::fs::remove_file(&ours_path);
    }
    assert!(failed.is_empty(), "failed: {failed:?}");
}
