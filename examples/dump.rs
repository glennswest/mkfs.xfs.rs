//! Print every metadata field of an XFS image, one per line.
//!
//!   cargo run --example dump -- fs.img

use mkfs_xfs::device::FileDevice;
use mkfs_xfs::inspect::dump;

#[tokio::main]
async fn main() -> mkfs_xfs::Result<()> {
    let path = std::env::args().nth(1).expect("usage: dump IMAGE");
    let dev = FileDevice::open(&path).await?;
    print!("{}", dump(&dev).await?.to_text());
    Ok(())
}
