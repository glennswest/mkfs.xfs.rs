//! How the formatter uses a device.

use std::time::Instant;

use mkfs_xfs::device::{BlockDevice, MemDevice};
use mkfs_xfs::format::format;
use mkfs_xfs::geometry::Params;
use mkfs_xfs::inspect::dump;

/// A device that enforces its 4 KiB logical sector refuses anything else
/// (mkfs.ext4.rs#5). The formatter and the reader keep to whole blocks at
/// block boundaries, so both work on it — and the sector size comes from
/// the device, as mkfs.xfs takes it.
#[tokio::test]
async fn whole_block_io_on_a_strict_4k_device() {
    let dev = MemDevice::strict(2 << 30, 4096);
    let r = format(&dev, &Params::new()).await.expect("format on a strict 4 KiB device");
    assert_eq!(r.geometry.sectsize, 4096);
    let d = dump(&dev).await.expect("read back on a strict 4 KiB device");
    assert_eq!(d.get("ag[0].sb.sectsize"), Some("4096"));
    assert_eq!(d.get("ag[0].sb.logsunit"), Some("4096"));
}

/// A petabyte formats in memory: what is written is headers, btree roots,
/// one inode chunk and one log record, not the device.
#[tokio::test]
async fn a_petabyte_formats_quickly_and_sparsely() {
    let dev = MemDevice::new(1 << 50);
    let t = Instant::now();
    let r = format(&dev, &Params::new()).await.unwrap();
    let took = t.elapsed();
    println!(
        "1 PiB: {} AGs, log {} blocks, formatted in {took:?}, {} KiB resident",
        r.geometry.agcount,
        r.geometry.logblocks,
        dev.resident() >> 10
    );
    assert_eq!(r.geometry.agcount, 1024);
    assert!(dev.resident() < 256 << 20, "resident {}", dev.resident());
    let mut sb = vec![0u8; 4096];
    dev.read_at(0, &mut sb).await.unwrap();
    assert_eq!(&sb[..4], b"XFSB");
}

/// Finding 852fbf8ad41a (#22): `FileDevice` does its I/O on the blocking
/// pool, so awaiting it releases the runtime worker. On a current-thread
/// runtime a task spawned beforehand can only run if the I/O yields; before
/// the fix every call ran to completion inside one poll and it never did.
/// The data still round-trips, and `write_zeroes` still zeroes.
#[tokio::test(flavor = "current_thread")]
async fn file_device_io_yields_the_runtime_worker() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use mkfs_xfs::device::FileDevice;

    const LEN: usize = 16 << 20;
    let dir = tempfile::tempdir().unwrap();
    let dev = FileDevice::create(dir.path().join("yield.img"), 2 * LEN as u64).await.unwrap();

    // Spawn a task that marks itself run, then return whether it has.
    let ran = |flag: &Arc<AtomicBool>| {
        flag.store(false, Ordering::SeqCst);
        let f = Arc::clone(flag);
        tokio::spawn(async move { f.store(true, Ordering::SeqCst) });
    };
    let flag = Arc::new(AtomicBool::new(false));

    let data: Vec<u8> = (0..LEN).map(|i| (i % 251) as u8 + 1).collect();
    ran(&flag);
    dev.write_at(LEN as u64, &data).await.unwrap();
    assert!(flag.load(Ordering::SeqCst), "write_at ran without yielding");

    let mut back = vec![0u8; LEN];
    ran(&flag);
    dev.read_at(LEN as u64, &mut back).await.unwrap();
    assert!(flag.load(Ordering::SeqCst), "read_at ran without yielding");
    assert!(back == data, "read_at returned other bytes than written");

    ran(&flag);
    dev.write_zeroes(LEN as u64, LEN as u64).await.unwrap();
    assert!(flag.load(Ordering::SeqCst), "write_zeroes ran without yielding");
    dev.read_at(LEN as u64, &mut back).await.unwrap();
    assert!(back.iter().all(|&b| b == 0), "write_zeroes left data");

    dev.flush().await.unwrap();
}

/// Finding 12ddb7cbd34e (#22): an image too short to hold a superblock
/// header — empty, or one byte — is an error from the reader (and so from
/// `xfs-admin` and `dump`), not a panic. The check itself came with #20;
/// this holds it through a `FileDevice`, as the finding describes.
#[tokio::test]
async fn empty_and_one_byte_images_are_not_xfs() {
    use mkfs_xfs::device::FileDevice;
    use mkfs_xfs::error::Error;
    use mkfs_xfs::inspect::geometry;

    let dir = tempfile::tempdir().unwrap();
    let empty = FileDevice::create(dir.path().join("empty.img"), 0).await.unwrap();
    assert!(matches!(geometry(&empty).await, Err(Error::NotXfs(_))));
    assert!(matches!(dump(&empty).await, Err(Error::NotXfs(_))));
    assert!(matches!(mkfs_xfs::admin::set_uuid(&empty, [7; 16]).await, Err(Error::NotXfs(_))));

    let one = FileDevice::create(dir.path().join("one.img"), 1).await.unwrap();
    one.write_at(0, b"X").await.unwrap();
    assert!(matches!(geometry(&one).await, Err(Error::NotXfs(_))));
    assert!(matches!(mkfs_xfs::admin::set_label(&one, "x").await, Err(Error::NotXfs(_))));
}
