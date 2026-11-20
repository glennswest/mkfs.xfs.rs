//! The CLI never destroys a target it was told not to write (`-N`), never
//! truncates before refusing bad parameters, and refuses sizes that do not
//! fit 32 bits instead of wrapping them (issue #19).

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SENTINEL: &[u8] = b"sentinel: this file must survive";

fn sentinel(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_safety");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, SENTINEL).unwrap();
    path
}

fn mkfs(args: &[&str], target: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mkfs-xfs")).args(args).arg(target).output().unwrap()
}

fn assert_untouched(path: &Path) {
    assert_eq!(std::fs::read(path).unwrap(), SENTINEL, "{} was changed", path.display());
}

/// Findings 706e2b7cf06c / 1780bbcd6dfa: `-N --create` reports the geometry
/// of the size asked for and leaves an existing file as it was.
#[test]
fn dry_run_create_leaves_existing_file() {
    let img = sentinel("dry-create");
    let out = mkfs(&["-N", "--create", "1g"], &img);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("blocks=262144,"), "{text}");
    assert_untouched(&img);
}

/// `-N --create` on a path that does not exist does not make it.
#[test]
fn dry_run_create_makes_no_file() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_safety");
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("dry-create-absent");
    let _ = std::fs::remove_file(&img);
    let out = mkfs(&["-N", "--create", "1g"], &img);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!img.exists());
}

/// A dry run whose geometry is refused leaves the target alone too.
#[test]
fn dry_run_create_with_bad_geometry_leaves_file() {
    let img = sentinel("dry-create-bad");
    let out = mkfs(&["-N", "-b", "size=3000", "--create", "1g"], &img);
    assert!(!out.status.success());
    assert_untouched(&img);
}

/// A real `--create` with parameters the geometry refuses does not truncate.
#[test]
fn create_refuses_bad_geometry_before_truncating() {
    let img = sentinel("create-bad");
    let out = mkfs(&["-b", "size=3000", "--create", "1g"], &img);
    assert!(!out.status.success());
    assert_untouched(&img);
}

/// `-N` on an existing image (no --create) reads it and changes nothing.
#[test]
fn dry_run_existing_image_is_not_written() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_safety");
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("dry-existing");
    std::fs::File::create(&img).unwrap().set_len(1 << 30).unwrap();
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
        f.write_all(SENTINEL).unwrap();
    }
    let out = mkfs(&["-N"], &img);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let head = std::fs::read(&img).unwrap();
    assert_eq!(&head[..SENTINEL.len()], SENTINEL);
    assert_eq!(head.len() as u64, 1 << 30);
}

/// Finding d8ad2d03becd: a block, sector or inode size over 32 bits is
/// refused; 4294971392 = 2^32 + 4096 once wrapped to 4096.
#[test]
fn sizes_over_32_bits_are_refused() {
    for opt in ["-b", "-s", "-i"] {
        let img = sentinel(&format!("wrap{opt}"));
        let out = mkfs(&["-N", opt, "size=4294971392", "--create", "1g"], &img);
        assert!(!out.status.success(), "{opt} size=4294971392 was accepted");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("does not fit 32 bits"), "{opt}: {err}");
        assert_untouched(&img);
    }
}
