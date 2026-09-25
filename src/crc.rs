//! CRC32C (Castagnoli), the checksum on every v5 metadata structure.
//!
//! XFS checksums a structure with its own CRC field treated as zero, stores
//! the result little-endian, and — unlike most users of CRC32C — keeps the
//! final inversion (`xfs_start_cksum_safe` / `xfs_end_cksum`).

const fn table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0x82f6_3b78 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static TABLE: [u32; 256] = table();

fn update(mut c: u32, data: &[u8]) -> u32 {
    for &b in data {
        c = TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c
}

/// CRC32C of `data`.
pub fn crc32c(data: &[u8]) -> u32 {
    !update(!0, data)
}

/// The checksum of `buf` as XFS computes it: the four bytes at `off` (the
/// structure's own CRC field) read as zero.
pub fn cksum(buf: &[u8], off: usize) -> u32 {
    let c = update(!0, &buf[..off]);
    let c = update(c, &[0; 4]);
    !update(c, &buf[off + 4..])
}

/// Compute and store the checksum of `buf` at `off`.
pub fn stamp(buf: &mut [u8], off: usize) {
    let c = cksum(buf, off);
    buf[off..off + 4].copy_from_slice(&c.to_le_bytes());
}

/// Whether the checksum stored at `off` matches the rest of `buf`.
pub fn verify(buf: &[u8], off: usize) -> bool {
    buf.len() >= off + 4 && crate::bytes::le32(buf, off) == cksum(buf, off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value() {
        // The standard CRC-32C check value.
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn stamp_then_verify() {
        let mut buf = vec![7u8; 64];
        stamp(&mut buf, 8);
        assert!(verify(&buf, 8));
        buf[20] ^= 1;
        assert!(!verify(&buf, 8));
    }
}
