# CLAUDE.md — mkfs.xfs.rs

XFS formatter and checker in pure Rust. Follow mkfs.ext4.rs's structure and
rules (read its CLAUDE.md): from the on-disk spec, held to real `mkfs.xfs`
output by a comparison tool and golden tests, async block I/O, no kernel.

## Work plan

### In progress — issue #1: format a valid XFS filesystem, held to real mkfs.xfs
Plan (2026-09-25):
- Modules after mkfs-ext4: `device` (async `BlockDevice`, file/memory),
  `crc`, `structs` (superblock, AGF, AGI, AGFL, short-form B+tree blocks,
  dinode, short-form dir, log record header), `geometry` (mkfs.xfs's default
  calculations: AG count/size, log size/placement, inode alignment, sunit),
  `format` (async, AGs written in parallel), `compare` (field-level diff of
  two XFS images: structural / identity / incidental), CLI `mkfs-xfs`.
- Reference: real mkfs.xfs on dev.g8.lo (run as the unprivileged build user
  through `sc-build`), images captured with a pinned UUID and vendored under
  `tests/golden/`; golden tests fail on any structural difference.
- Validity on dev: `xfs_repair -n -f` on our images, as the build user (no root).
- [ ] Superblock, AG headers (AGF/AGI/AGFL), free-space and inode B+trees, root inode, log (v5/CRC; reflink and finobt on, as current mkfs.xfs defaults)
- [ ] Geometry: AG count/size, sector/block/inode sizes, stripe (su/sw) — matching mkfs.xfs defaults for a given size
- [ ] Lazy formatting where XFS allows (sparse inode chunks), so a PB filesystem formats quickly
- [ ] Comparison tool vs real mkfs.xfs (xfs_db dumps) and golden tests, run on dev
- [ ] Checker (an `xfs_repair -n` subset): structural validation
- [ ] Large sizes: 16 TiB … 1 PiB (measured with stormcos's emulated large drives)
