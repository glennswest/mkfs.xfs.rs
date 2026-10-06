//! `admin::set_uuid` / `set_label` against the `xfs_admin` installed here,
//! when there is one (dev.g8.lo has xfsprogs 6.15.0; elsewhere these tests
//! report a skip).
//!
//! Each case: real `mkfs.xfs` formats an image and it is copied. Then, step
//! by step, `xfs_admin` changes one copy and this crate the other, and the
//! two images must be identical byte for byte — superblocks, log and all —
//! after every step. The steps walk the log through its cases: the fresh
//! one-record log (cycle 1), a log written all the way round (cycles 2 and
//! 3), and setting the UUID back to the metadata UUID. `xfs_repair -n` must
//! pass ours at the end.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use mkfs_xfs::admin;
use mkfs_xfs::device::FileDevice;

const ORIG: &str = "12345678-1234-5678-9abc-123456789abc";

fn have(tool: &str) -> bool {
    Command::new(tool).arg("-V").output().map(|o| o.status.success()).unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("admin");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn uuid(s: &str) -> [u8; 16] {
    *uuid::Uuid::parse_str(s).unwrap().as_bytes()
}

/// The first offset where the two files differ.
fn first_difference(a: &Path, b: &Path) -> Option<u64> {
    let (mut fa, mut fb) = (std::fs::File::open(a).unwrap(), std::fs::File::open(b).unwrap());
    let (mut ba, mut bb) = (vec![0u8; 1 << 22], vec![0u8; 1 << 22]);
    let mut at = 0u64;
    loop {
        let na = fa.read(&mut ba).unwrap();
        let mut nb = 0;
        while nb < na {
            let n = fb.read(&mut bb[nb..na]).unwrap();
            if n == 0 {
                break;
            }
            nb += n;
        }
        if na != nb {
            return Some(at + nb as u64);
        }
        if na == 0 {
            return None;
        }
        if let Some(i) = ba[..na].iter().zip(&bb[..na]).position(|(x, y)| x != y) {
            return Some(at + i as u64);
        }
        at += na as u64;
    }
}

enum Step {
    Uuid(&'static str),
    Label(&'static str),
}

/// (name, size, mkfs.xfs options)
fn cases() -> Vec<(&'static str, u64, Vec<&'static str>)> {
    const G: u64 = 1 << 30;
    vec![
        ("1g", G, vec![]),
        ("1g-b1024", G, vec!["-b", "size=1024"]),
        ("2g-s4096", 2 * G, vec!["-s", "size=4096"]),
        ("3g-agcount9-b8192", 3 * G, vec!["-d", "agcount=9", "-b", "size=8192"]),
        // Log stripe units: a 64-block first record, and one that needs
        // extended record headers.
        ("1g-lsu32k", G, vec!["-l", "su=32768"]),
        ("2g-lsu256k", 2 * G, vec!["-l", "su=262144"]),
        ("777m-odd", 777 * (1 << 20) + 12345, vec![]),
    ]
}

#[tokio::test]
async fn ours_match_xfs_admin_byte_for_byte() {
    if !have("mkfs.xfs") || !have("xfs_admin") {
        println!("SKIP: no mkfs.xfs / xfs_admin on PATH");
        return;
    }
    let steps = [
        Step::Label("first"),
        Step::Uuid("11111111-2222-3333-4444-555555555555"),
        Step::Uuid("66666666-7777-8888-9999-aaaaaaaaaaaa"),
        Step::Label("twelve-bytes"),
        Step::Uuid(ORIG),
        Step::Uuid("bbbbbbbb-cccc-dddd-eeee-ffffffffffff"),
        Step::Label(""),
    ];
    let mut failed = Vec::new();
    'case: for (name, size, opts) in cases() {
        let theirs = scratch(&format!("{name}.theirs"));
        let ours = scratch(&format!("{name}.ours"));
        let _ = std::fs::remove_file(&theirs);
        let f = std::fs::File::create(&theirs).unwrap();
        f.set_len(size).unwrap();
        drop(f);
        let st = Command::new("mkfs.xfs")
            .args(["-q", "-f", "-m", &format!("uuid={ORIG}")])
            .args(&opts)
            .arg(&theirs)
            .status()
            .unwrap();
        assert!(st.success(), "{name}: mkfs.xfs failed");
        let st = Command::new("cp").arg("--sparse=always").arg(&theirs).arg(&ours).status().unwrap();
        assert!(st.success());

        for (n, step) in steps.iter().enumerate() {
            let (what, arg) = match step {
                Step::Uuid(u) => ("-U", u.to_string()),
                Step::Label(l) => ("-L", if l.is_empty() { "--".to_string() } else { l.to_string() }),
            };
            let out = Command::new("xfs_admin").args([what, &arg]).arg(&theirs).output().unwrap();
            assert!(out.status.success(), "{name}: xfs_admin {what} {arg}: {}",
                String::from_utf8_lossy(&out.stderr));
            let dev = FileDevice::open(&ours).await.unwrap();
            let r = match step {
                Step::Uuid(u) => admin::set_uuid(&dev, uuid(u)).await,
                Step::Label(l) => admin::set_label(&dev, l).await,
            };
            drop(dev);
            if let Err(e) = r {
                println!("{name}: step {n} ({what} {arg}): {e}");
                failed.push(format!("{name} step {n}"));
                continue 'case;
            }
            if let Some(at) = first_difference(&ours, &theirs) {
                println!("{name}: step {n} ({what} {arg}): first difference from xfs_admin at byte {at}");
                failed.push(format!("{name} step {n}"));
                continue 'case;
            }
        }
        if have("xfs_repair") {
            let out = Command::new("xfs_repair").args(["-n", "-f"]).arg(&ours).output().unwrap();
            if !out.status.success() {
                println!("{name}: xfs_repair -n failed:\n{}", String::from_utf8_lossy(&out.stdout));
                failed.push(format!("{name} (xfs_repair)"));
                continue;
            }
        }
        println!("{name}: {} steps identical to xfs_admin, xfs_repair -n clean", steps.len());
        let _ = std::fs::remove_file(&theirs);
        let _ = std::fs::remove_file(&ours);
    }
    assert!(failed.is_empty(), "failed: {failed:?}");
}

#[tokio::test]
async fn restore_matches_xfs_admin() {
    if !have("mkfs.xfs") || !have("xfs_admin") {
        println!("SKIP: no mkfs.xfs / xfs_admin on PATH");
        return;
    }
    let theirs = scratch("restore.theirs");
    let ours = scratch("restore.ours");
    let _ = std::fs::remove_file(&theirs);
    std::fs::File::create(&theirs).unwrap().set_len(1 << 30).unwrap();
    let st = Command::new("mkfs.xfs").args(["-q", "-f", "-m", &format!("uuid={ORIG}")]).arg(&theirs).status().unwrap();
    assert!(st.success());
    // Unchanged: nothing to restore, nothing written.
    Command::new("cp").arg("--sparse=always").arg(&theirs).arg(&ours).status().unwrap();
    assert!(!admin::restore_uuid(&FileDevice::open(&ours).await.unwrap()).await.unwrap());
    assert_eq!(first_difference(&ours, &theirs), None);

    for img in [&theirs, &ours] {
        let st = Command::new("xfs_admin").args(["-U", "generate"]).arg(img).output().unwrap();
        assert!(st.status.success());
    }
    // Different random UUIDs, same log cycle: restoring both converges.
    let st = Command::new("xfs_admin").args(["-U", "restore"]).arg(&theirs).output().unwrap();
    assert!(st.status.success());
    assert!(admin::restore_uuid(&FileDevice::open(&ours).await.unwrap()).await.unwrap());
    assert_eq!(first_difference(&ours, &theirs), None, "restore differs from xfs_admin -U restore");
    let _ = std::fs::remove_file(&theirs);
    let _ = std::fs::remove_file(&ours);
}
