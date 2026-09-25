# CLAUDE.md — mkfs.xfs.rs

XFS formatter and checker in pure Rust. Follow mkfs.ext4.rs's structure and
rules (read its CLAUDE.md): from the on-disk spec, held to real `mkfs.xfs`
output by a comparison tool and golden tests, async block I/O, no kernel.

## Work plan
- [ ] Superblock, AG headers (AGF/AGI/AGFL), free-space and inode B+trees, root inode, log (v5/CRC; reflink and finobt on, as current mkfs.xfs defaults)
- [ ] Geometry: AG count/size, sector/block/inode sizes, stripe (su/sw) — matching mkfs.xfs defaults for a given size
- [ ] Lazy formatting where XFS allows (sparse inode chunks), so a PB filesystem formats quickly
- [ ] Comparison tool vs real mkfs.xfs (xfs_db dumps) and golden tests, run on dev
- [ ] Checker (an `xfs_repair -n` subset): structural validation
- [ ] Large sizes: 16 TiB … 1 PiB (measured with stormcos's emulated large drives)
