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
