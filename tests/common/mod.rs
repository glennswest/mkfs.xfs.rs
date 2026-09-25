//! Shared by the integration tests.
#![allow(dead_code)]

use std::io::Read;

use mkfs_xfs::device::{BlockDevice, MemDevice};

/// The UUID every golden image was pinned to.
pub const GOLDEN_UUID: [u8; 16] = [
    0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
];

/// Load `tests/golden/NAME.sparse.gz` into a sparse memory device.
pub async fn golden(name: &str) -> MemDevice {
    let path = format!("{}/tests/golden/{name}.sparse.gz", env!("CARGO_MANIFEST_DIR"));
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("opening {path}: {e}"));
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(file)
        .read_to_end(&mut raw)
        .unwrap_or_else(|e| panic!("decompressing {path}: {e}"));
    assert_eq!(&raw[..8], b"XFSSPRS1", "{path}: not a sparse image");
    let size = u64::from_be_bytes(raw[8..16].try_into().unwrap());
    let dev = MemDevice::new(size);
    let mut at = 16;
    while at < raw.len() {
        let off = u64::from_be_bytes(raw[at..at + 8].try_into().unwrap());
        let len = u32::from_be_bytes(raw[at + 8..at + 12].try_into().unwrap()) as usize;
        dev.write_at(off, &raw[at + 12..at + 12 + len]).await.unwrap();
        at += 12 + len;
    }
    dev
}
