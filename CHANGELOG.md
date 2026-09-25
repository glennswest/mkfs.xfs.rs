# Changelog

## [Unreleased]

### 2026-09-25
- **feat:** XFS formatter (#1): superblocks, AGF/AGI/AGFL, free-space, inode,
  free-inode, reverse-mapping and refcount btree roots, the root inode chunk
  (root directory, realtime bitmap and summary inodes) and the log — the end
  state mkfs.xfs 6.15 leaves, written once, allocation groups in parallel.
- **feat:** `geometry` — mkfs.xfs's default calculations: sector size from the
  device's physical sector, AG count and size (`calc_default_ag_geometry`),
  imaxpct, log size and placement, rt extent size, log stripe unit.
- **feat:** `inspect` and `compare` — read every metadata field of a v5 XFS
  filesystem and diff two of them, classing each difference as structural,
  identity (UUID) or incidental (timestamps, generation numbers, their CRCs).
- **feat:** `device` — async `BlockDevice`, `FileDevice` (punches holes for
  zeroes in image files), sparse `MemDevice` (1 PiB formats in memory).
- **feat:** `mkfs-xfs` CLI with mkfs.xfs option syntax; `compare` and `dump`
  examples.
- **fix:** rt extent size defaults to 4 KiB (4 blocks at 1 KiB blocks), and a
  log sector over 512 bytes gets a one-block log stripe unit, as mkfs.xfs sets
  them.
- **test:** 12 golden images from mkfs.xfs 6.15.0, 300 MB to 1 PiB, all
  field-identical; a live comparison with `xfs_repair -n` up to 8 TiB; a
  kernel mount/write/remount test in a VM (`tests/kernel-mount.sh`), no root.
- **docs:** README, CLAUDE.md work plan; follow-ups filed as #2–#5.
- **chore:** Scaffold — the XFS sibling of the ext4 crate (owner: "add XFS to our formatting/mkfs and import tools as well as ext4")
