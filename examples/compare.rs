//! Diff two XFS images field by field.
//!
//!   cargo run --example compare -- ours.img theirs.img [--all]
//!
//! Prints every structural difference (and, with --all, the identity and
//! incidental ones too). Exits 1 if any difference is structural.

use mkfs_xfs::compare::{compare, structural};
use mkfs_xfs::device::FileDevice;

#[tokio::main]
async fn main() -> mkfs_xfs::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().any(|a| a == "--all");
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if paths.len() != 2 {
        eprintln!("usage: compare OURS THEIRS [--all]");
        std::process::exit(2);
    }
    let ours = FileDevice::open(paths[0]).await?;
    let theirs = FileDevice::open(paths[1]).await?;
    let diffs = compare(&ours, &theirs).await?;
    let bad = structural(&diffs);
    for d in &diffs {
        if all || d.class == mkfs_xfs::inspect::Class::Structural {
            println!("{d}");
        }
    }
    println!(
        "{} differences, {} structural",
        diffs.len(),
        bad.len()
    );
    std::process::exit(i32::from(!bad.is_empty()));
}
