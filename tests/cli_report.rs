//! The `mkfs-xfs -N` report against the image it describes and, where it
//! is installed, against `mkfs.xfs -N` (dev.g8.lo has xfsprogs; elsewhere
//! that comparison reports a skip).

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::Command;

const SIZE: u64 = 1 << 30;

fn image(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_report");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::File::create(&path).unwrap().set_len(SIZE).unwrap();
    path
}

/// The two `log` lines of a `-N` report.
fn log_lines(tool: &str, opts: &[&str], img: &Path) -> Vec<String> {
    let out = Command::new(tool).arg("-N").args(opts).arg(img).output().unwrap();
    assert!(out.status.success(), "{tool} -N {opts:?}: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|l| l.starts_with("log ")).expect("no log line");
    lines[at..at + 2].iter().map(|l| l.to_string()).collect()
}

/// (name, options, the log stripe unit in blocks)
const CASES: &[(&str, &[&str], u32)] = &[
    ("default", &[], 0),
    ("s4096", &["-s", "size=4096"], 1),
    ("b1024", &["-b", "size=1024"], 0),
    ("b1024-s1024", &["-b", "size=1024", "-s", "size=1024"], 1),
    ("b8192-s4096", &["-b", "size=8192", "-s", "size=4096"], 1),
];

#[test]
fn log_sunit_is_one_block_for_log_sectors_over_512() {
    for (name, opts, sunit) in CASES {
        let img = image(&format!("ours-{name}"));
        let lines = log_lines(env!("CARGO_BIN_EXE_mkfs-xfs"), opts, &img);
        assert!(lines[1].contains(&format!(" sunit={sunit} blks,")), "{name}: {}", lines[1]);
    }
}

#[test]
fn log_lines_match_mkfs_xfs() {
    if !Command::new("mkfs.xfs").arg("-V").output().map(|o| o.status.success()).unwrap_or(false) {
        println!("SKIP: no mkfs.xfs on PATH");
        return;
    }
    for (name, opts, _) in CASES {
        let ours = log_lines(env!("CARGO_BIN_EXE_mkfs-xfs"), opts, &image(&format!("ours-{name}")));
        let theirs = log_lines("mkfs.xfs", opts, &image(&format!("theirs-{name}")));
        assert_eq!(ours, theirs, "{name}");
    }
}
