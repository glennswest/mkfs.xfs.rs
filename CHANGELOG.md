# Changelog

## [Unreleased]
<!-- New unreleased changes go here -->

### 2026-09-28
- **docs:** Rechecked README, CLAUDE.md and crate docs against the code (no
  code change since v0.2.0): CLI options, 128 KiB overwrite clearing, 12-byte
  label limit, the `-N` log `sunit=0` (#9) and stormblock's `v0.2.0` pin all
  match. Nothing new promised that the code does not do; open gaps stay #2–#9.

### 2026-09-27
- **docs:** Refreshed from the code: the crate description no longer promises
  a checker (filed as #7); README documents `-f`, `-m crc=1`, size suffixes and
  how it ships (git tags, no service or config, stormblock as consumer, not a
  stormcentral component); the concurrency=0 equivalence is marked untested;
  known gaps link #2–#7.
- **docs:** README checked against the code again. It now covers the
  library-side behaviour it left out: `write_zeroes` and when to override it,
  `Params` fields with no builder, reproducible formats through
  `uuid`/`time`/`gen_seed`, and the 12-byte label limit.
- **docs:** Third check of README, CLAUDE.md and crate docs against the code
  (CLI options and suffixes, refusals, overwrite clearing, `write_zeroes`,
  stormblock's `v0.2.0` pin): accurate. The `-N` report's hardcoded log
  `sunit=0` is filed as #9 and noted in the README.
- **docs:** CLAUDE.md work plan lists #9 and points #3's owner decision at
  #8; no code has changed since the last check, so the README stands.

## [v0.2.0] — 2026-09-25

### Added
- XFS formatter (#1): superblocks, AGF/AGI/AGFL, free-space, inode,
  free-inode, reverse-mapping and refcount btree roots, the root inode chunk
  (root directory, realtime bitmap and summary inodes) and the log — the end
  state mkfs.xfs 6.15 leaves, written once, allocation groups in parallel.
- `geometry` — mkfs.xfs's default calculations: sector size from the device's
  physical sector, AG count and size (`calc_default_ag_geometry`), imaxpct,
  log size and placement, rt extent size, log stripe unit.
- `inspect` and `compare` — read every metadata field of a v5 XFS filesystem
  and diff two of them, classing each difference as structural, identity
  (UUID) or incidental (timestamps, generation numbers, their CRCs).
- `device` — async `BlockDevice`, `FileDevice` (punches holes for zeroes in
  image files), sparse `MemDevice` (1 PiB formats in memory).
- `mkfs-xfs` CLI with mkfs.xfs option syntax; `compare` and `dump` examples.
- Tests: 12 golden images from mkfs.xfs 6.15.0, 300 MB to 1 PiB, all
  field-identical; a live comparison with `xfs_repair -n` up to 8 TiB; a
  kernel mount/write/remount test in a VM (`tests/kernel-mount.sh`), no root.

### Fixed
- rt extent size defaults to 4 KiB (4 blocks at 1 KiB blocks), and a log
  sector over 512 bytes gets a one-block log stripe unit, as mkfs.xfs sets
  them.

### Documentation
- README, CLAUDE.md work plan; follow-ups filed as #2–#5.

### Changed
- Scaffold — the XFS sibling of the ext4 crate (owner: "add XFS to our
  formatting/mkfs and import tools as well as ext4").
