//! The device seam.
//!
//! Everything this crate writes goes through [`BlockDevice`]. The trait takes
//! `&self` for reads *and* writes, so a format fans out: allocation groups are
//! disjoint byte ranges, and writing them concurrently needs positional I/O,
//! not exclusion.
//!
//! **Every operation this crate issues is whole filesystem blocks at a block
//! boundary** (mkfs.ext4.rs#5 is why: a device that enforces its logical
//! block refuses anything smaller, and a loop device hides that). A block is
//! never smaller than a sector, so every operation is whole sectors too. The
//! one exception is [`crate::inspect`] reading the first sector of a device
//! before it knows the block size.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::Mutex;

use crate::error::{Error, Result};

/// A device this crate can lay a filesystem onto.
///
/// Implementations must be safe to call concurrently from many tasks; writes
/// to disjoint ranges must not interfere.
#[async_trait::async_trait]
pub trait BlockDevice: Send + Sync {
    /// Total addressable size in bytes.
    fn size(&self) -> u64;

    /// The smallest I/O the device accepts. Defaults to 512.
    fn logical_sector_size(&self) -> u32 {
        512
    }

    /// The device's physical sector size, which `mkfs.xfs` uses as the
    /// filesystem's sector size by default (a 512e drive gets 4096-byte XFS
    /// sectors). Defaults to the logical sector size.
    fn physical_sector_size(&self) -> u32 {
        self.logical_sector_size()
    }

    /// Read exactly `buf.len()` bytes starting at `offset`.
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;

    /// Write all of `buf` starting at `offset`.
    async fn write_at(&self, offset: u64, buf: &[u8]) -> Result<()>;

    /// Flush buffered writes to stable storage.
    async fn flush(&self) -> Result<()>;

    /// Make `len` bytes at `offset` read back as zero.
    ///
    /// The default writes zeroes in 1 MiB chunks. A device with a
    /// write-zeroes or discard-that-zeroes primitive should override it: the
    /// log alone is up to 2 GiB, and `mkfs.xfs` zeroes it the same way.
    async fn write_zeroes(&self, offset: u64, len: u64) -> Result<()> {
        const CHUNK: u64 = 1 << 20;
        let zeroes = vec![0u8; CHUNK.min(len).max(1) as usize];
        let mut done = 0u64;
        while done < len {
            let n = (len - done).min(zeroes.len() as u64) as usize;
            self.write_at(offset + done, &zeroes[..n]).await?;
            done += n as u64;
        }
        Ok(())
    }
}

/// Bounds-check a request against a device size.
pub(crate) fn check_bounds(offset: u64, len: u64, size: u64) -> Result<()> {
    match offset.checked_add(len) {
        Some(end) if end <= size => Ok(()),
        _ => Err(Error::OutOfBounds { offset, len, size }),
    }
}

/// A device backed by a file or a raw block device, with positional I/O so
/// concurrent writes to disjoint ranges do not contend on a cursor.
pub struct FileDevice {
    file: std::fs::File,
    size: u64,
    logical: u32,
    physical: u32,
    regular: bool,
}

impl FileDevice {
    /// Open an existing file or block device.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = tokio::task::spawn_blocking(move || {
            std::fs::OpenOptions::new().read(true).write(true).open(path)
        })
        .await
        .map_err(|e| Error::io(0, io::Error::other(e)))?
        .map_err(|e| Error::io(0, e))?;
        let meta = file.metadata().map_err(|e| Error::io(0, e))?;
        let regular = meta.is_file();
        let size = if regular { meta.len() } else { seek_size(&file)? };
        let (logical, physical) = sector_sizes(&file, regular);
        Ok(Self { file, size, logical, physical, regular })
    }

    /// Create (or truncate) a sparse file of exactly `size` bytes.
    pub async fn create(path: impl AsRef<Path>, size: u64) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = tokio::task::spawn_blocking(move || {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;
            f.set_len(size)?;
            Ok::<_, io::Error>(f)
        })
        .await
        .map_err(|e| Error::io(0, io::Error::other(e)))?
        .map_err(|e| Error::io(0, e))?;
        Ok(Self { file, size, logical: 512, physical: 512, regular: true })
    }

    /// Declare the sector size of the device an image is being built for. A
    /// plain file has none of its own.
    pub fn with_sector_size(mut self, sector_size: u32) -> Self {
        self.logical = sector_size;
        self.physical = sector_size;
        self
    }
}

fn seek_size(file: &std::fs::File) -> Result<u64> {
    use std::io::{Seek, SeekFrom};
    let mut f = file.try_clone().map_err(|e| Error::io(0, e))?;
    f.seek(SeekFrom::End(0)).map_err(|e| Error::io(0, e))
}

/// Logical and physical sector sizes, asked of the kernel for a block
/// device. A regular file reports 512 for both, as `mkfs.xfs` assumes.
fn sector_sizes(file: &std::fs::File, regular: bool) -> (u32, u32) {
    #[cfg(target_os = "linux")]
    if !regular {
        let logical = rustix::fs::ioctl_blksszget(file).unwrap_or(512).max(512);
        let physical = rustix::fs::ioctl_blkpbszget(file).unwrap_or(logical).max(logical);
        return (logical, physical);
    }
    let _ = (file, regular);
    (512, 512)
}

#[async_trait::async_trait]
impl BlockDevice for FileDevice {
    fn size(&self) -> u64 {
        self.size
    }

    fn logical_sector_size(&self) -> u32 {
        self.logical
    }

    fn physical_sector_size(&self) -> u32 {
        self.physical
    }

    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        use std::os::unix::fs::FileExt;
        check_bounds(offset, buf.len() as u64, self.size)?;
        self.file.read_exact_at(buf, offset).map_err(|e| Error::io(offset, e))
    }

    async fn write_at(&self, offset: u64, buf: &[u8]) -> Result<()> {
        use std::os::unix::fs::FileExt;
        check_bounds(offset, buf.len() as u64, self.size)?;
        self.file.write_all_at(buf, offset).map_err(|e| Error::io(offset, e))
    }

    async fn flush(&self) -> Result<()> {
        self.file.sync_data().map_err(|e| Error::io(0, e))
    }

    async fn write_zeroes(&self, offset: u64, len: u64) -> Result<()> {
        check_bounds(offset, len, self.size)?;
        // A regular file punches the range out: it reads back as zeroes and
        // stays sparse, which is what an image file wants.
        #[cfg(target_os = "linux")]
        if self.regular {
            use rustix::fs::{fallocate, FallocateFlags};
            let flags = FallocateFlags::PUNCH_HOLE | FallocateFlags::KEEP_SIZE;
            if fallocate(&self.file, flags, offset, len).is_ok() {
                return Ok(());
            }
        }
        let zeroes = vec![0u8; (1u64 << 20).min(len).max(1) as usize];
        let mut done = 0u64;
        while done < len {
            let n = (len - done).min(zeroes.len() as u64) as usize;
            self.write_at(offset + done, &zeroes[..n]).await?;
            done += n as u64;
        }
        Ok(())
    }
}

/// Granularity of [`MemDevice`]'s sparse storage.
const CHUNK: u64 = 64 * 1024;

/// A sparse device held in memory: only chunks that were written hold
/// memory, so a 1 PiB filesystem formats into a few megabytes.
pub struct MemDevice {
    chunks: Mutex<BTreeMap<u64, Box<[u8]>>>,
    size: u64,
    logical: u32,
    physical: u32,
    strict: bool,
}

impl MemDevice {
    /// A zeroed device of `size` bytes with 512-byte sectors.
    pub fn new(size: u64) -> Self {
        Self::with_sector_size(size, 512)
    }

    /// A zeroed device that reports the given sector size.
    pub fn with_sector_size(size: u64, sector_size: u32) -> Self {
        Self {
            chunks: Mutex::new(BTreeMap::new()),
            size,
            logical: sector_size,
            physical: sector_size,
            strict: false,
        }
    }

    /// A device that refuses any I/O that is not whole sectors at a sector
    /// boundary, as a device enforcing its logical block does.
    pub fn strict(size: u64, sector_size: u32) -> Self {
        Self { strict: true, ..Self::with_sector_size(size, sector_size) }
    }

    fn check_aligned(&self, offset: u64, len: usize) -> Result<()> {
        let sector = u64::from(self.logical);
        if self.strict && (offset % sector != 0 || len as u64 % sector != 0) {
            return Err(Error::io(
                offset,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{len} bytes at offset {offset} is not whole {sector}-byte sectors"),
                ),
            ));
        }
        Ok(())
    }

    /// Every chunk holding a non-zero byte, as `(byte offset, contents)`.
    pub fn nonzero_chunks(&self) -> Vec<(u64, Vec<u8>)> {
        let chunks = self.chunks.lock().expect("mem device poisoned");
        chunks
            .iter()
            .filter(|(_, c)| c.iter().any(|&b| b != 0))
            .map(|(&i, c)| (i * CHUNK, c.to_vec()))
            .collect()
    }

    /// Bytes of memory the device holds.
    pub fn resident(&self) -> u64 {
        self.chunks.lock().expect("mem device poisoned").len() as u64 * CHUNK
    }

    fn copy_out(&self, offset: u64, buf: &mut [u8]) {
        let chunks = self.chunks.lock().expect("mem device poisoned");
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let (idx, within) = (pos / CHUNK, (pos % CHUNK) as usize);
            let n = (buf.len() - done).min(CHUNK as usize - within);
            match chunks.get(&idx) {
                Some(c) => buf[done..done + n].copy_from_slice(&c[within..within + n]),
                None => buf[done..done + n].fill(0),
            }
            done += n;
        }
    }

    fn copy_in(&self, offset: u64, buf: &[u8]) {
        let mut chunks = self.chunks.lock().expect("mem device poisoned");
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let (idx, within) = (pos / CHUNK, (pos % CHUNK) as usize);
            let n = (buf.len() - done).min(CHUNK as usize - within);
            let src = &buf[done..done + n];
            if src.iter().all(|&b| b == 0) {
                // Zeroes need no memory: drop a whole chunk, or zero part of
                // one that exists.
                if n == CHUNK as usize {
                    chunks.remove(&idx);
                } else if let Some(c) = chunks.get_mut(&idx) {
                    c[within..within + n].fill(0);
                }
            } else {
                let c = chunks
                    .entry(idx)
                    .or_insert_with(|| vec![0u8; CHUNK as usize].into_boxed_slice());
                c[within..within + n].copy_from_slice(src);
            }
            done += n;
        }
    }
}

#[async_trait::async_trait]
impl BlockDevice for MemDevice {
    fn size(&self) -> u64 {
        self.size
    }

    fn logical_sector_size(&self) -> u32 {
        self.logical
    }

    fn physical_sector_size(&self) -> u32 {
        self.physical
    }

    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        check_bounds(offset, buf.len() as u64, self.size)?;
        self.check_aligned(offset, buf.len())?;
        self.copy_out(offset, buf);
        Ok(())
    }

    async fn write_at(&self, offset: u64, buf: &[u8]) -> Result<()> {
        check_bounds(offset, buf.len() as u64, self.size)?;
        self.check_aligned(offset, buf.len())?;
        self.copy_in(offset, buf);
        Ok(())
    }

    async fn flush(&self) -> Result<()> {
        Ok(())
    }

    async fn write_zeroes(&self, offset: u64, len: u64) -> Result<()> {
        check_bounds(offset, len, self.size)?;
        self.check_aligned(offset, len as usize)?;
        let mut chunks = self.chunks.lock().expect("mem device poisoned");
        let end = offset + len;
        let first = offset / CHUNK;
        let last = end.div_ceil(CHUNK);
        let keys: Vec<u64> = chunks.range(first..last).map(|(&k, _)| k).collect();
        for idx in keys {
            let (cs, ce) = (idx * CHUNK, (idx + 1) * CHUNK);
            let (zs, ze) = (offset.max(cs), end.min(ce));
            if zs == cs && ze == ce {
                chunks.remove(&idx);
            } else if let Some(c) = chunks.get_mut(&idx) {
                c[(zs - cs) as usize..(ze - cs) as usize].fill(0);
            }
        }
        Ok(())
    }
}

/// `&D` is itself a device, so a caller can format a device it only borrowed.
#[async_trait::async_trait]
impl<D: BlockDevice + ?Sized> BlockDevice for &D {
    fn size(&self) -> u64 {
        (**self).size()
    }
    fn logical_sector_size(&self) -> u32 {
        (**self).logical_sector_size()
    }
    fn physical_sector_size(&self) -> u32 {
        (**self).physical_sector_size()
    }
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        (**self).read_at(offset, buf).await
    }
    async fn write_at(&self, offset: u64, buf: &[u8]) -> Result<()> {
        (**self).write_at(offset, buf).await
    }
    async fn flush(&self) -> Result<()> {
        (**self).flush().await
    }
    async fn write_zeroes(&self, offset: u64, len: u64) -> Result<()> {
        (**self).write_zeroes(offset, len).await
    }
}

/// `Arc<D>` is itself a device.
#[async_trait::async_trait]
impl<D: BlockDevice + ?Sized> BlockDevice for std::sync::Arc<D> {
    fn size(&self) -> u64 {
        (**self).size()
    }
    fn logical_sector_size(&self) -> u32 {
        (**self).logical_sector_size()
    }
    fn physical_sector_size(&self) -> u32 {
        (**self).physical_sector_size()
    }
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        (**self).read_at(offset, buf).await
    }
    async fn write_at(&self, offset: u64, buf: &[u8]) -> Result<()> {
        (**self).write_at(offset, buf).await
    }
    async fn flush(&self) -> Result<()> {
        (**self).flush().await
    }
    async fn write_zeroes(&self, offset: u64, len: u64) -> Result<()> {
        (**self).write_zeroes(offset, len).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mem_device_is_sparse() {
        let dev = MemDevice::new(1 << 50);
        dev.write_at((1 << 49) + 4096, &[7u8; 4096]).await.unwrap();
        assert_eq!(dev.resident(), CHUNK);
        let mut buf = vec![0u8; 8192];
        dev.read_at(1 << 49, &mut buf).await.unwrap();
        assert!(buf[..4096].iter().all(|&b| b == 0));
        assert!(buf[4096..].iter().all(|&b| b == 7));
        dev.write_zeroes(1 << 49, CHUNK).await.unwrap();
        assert_eq!(dev.resident(), 0);
    }

    #[tokio::test]
    async fn strict_refuses_partial_sectors() {
        let dev = MemDevice::strict(1 << 20, 4096);
        assert!(dev.write_at(512, &[0u8; 4096]).await.is_err());
        assert!(dev.write_at(4096, &[0u8; 512]).await.is_err());
        assert!(dev.write_at(4096, &[1u8; 4096]).await.is_ok());
    }
}
