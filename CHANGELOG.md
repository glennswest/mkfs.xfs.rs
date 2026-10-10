# Changelog

## [Unreleased]
<!-- New unreleased changes go here -->

### 2026-10-10
- **docs:** Rechecked README, CLAUDE.md and crate docs against the code
  for everything since 2026-10-02 (v0.2.1, v0.3.0: `admin`/`xfs-admin`,
  `tests/vm/`): CLI flags, `Params`, the stormblock API list, how it ships
  (git tags, no service, ports or config) all match. Removed the
  references to dev.g8.lo as where the tests run (retired 2026-10-07);
  the live and kernel tests run on sc-build's build VM and skip where
  xfsprogs is missing.

## [v0.3.0] — 2026-10-06

### Added
- `admin::set_uuid`, `admin::restore_uuid` and `admin::set_label` —
  `xfs_admin -U` / `-U restore` / `-L` over `BlockDevice`, leaving the
  device byte for byte as xfsprogs 6.15's `xfs_db` does: a clean-log check
  (`xlog_find_tail`, ported from libxlog; a dirty log is refused with the
  new `Error::NeedsRecovery`), the whole log rewritten under the new UUID
  at the next cycle (`libxfs_log_clear`), and every AG's superblock with
  `META_UUID` / `sb_meta_uuid` handled as `do_uuid` does. Also
  `admin::log_state`, and the `xfs-admin` binary (#6).
- `structs::log::record` and `lsn` — the general `libxfs_log_header`;
  `first_record` is built on it.

### Tests
- `tests/live_xfs_admin.rs` holds `admin` to `xfs_admin` byte for byte
  across log cycles 1 → 4, block and sector sizes and log stripe units;
  `tests/kernel-mount.sh` re-stamps the log the kernel left and mounts it
  (#6).
- `tests/vm/` — kernel verification in a throwaway VM through
  stormcentral's `testhost boot` (#11): `build-image.sh` makes a UEFI disk
  (Shell → the build box's kernel + a busybox initramfs with xfs/loop
  modules, xfs_repair and our binaries); `init.sh` formats 8 cases with
  mkfs-xfs, loop-mounts, writes, checks with `xfs_repair -n` and remounts,
  repeats after `xfs-admin -U`, and mounts two re-stamped clones of one
  blank side by side; prints `VERIFY PASS` / `VERIFY FAIL <why>`.

## [v0.2.1] — 2026-10-06

### Fixed
- The `-N` and post-format report printed the log `sunit=0 blks`
  always; it now prints the log stripe unit the filesystem has — one block
  when the log sector is over 512 bytes (`-s size=4096`, 512e/4Kn devices),
  as mkfs.xfs 6.15 does. The image was already right. New
  `tests/cli_report.rs` checks the report and compares its log lines with
  `mkfs.xfs -N` (#9).

### Documentation
- README and CLAUDE.md list every API stormblock uses (checked against
  stormblock `src/fs/xfs.rs`): the `BlockDevice` trait, `Error::io`,
  `Params::new().uuid()/.label()/.block_size()`, `format::format` and
  `Report`, `crc::verify`/`crc::stamp`, and `structs::sb::off`/`version`
  (#10).
- README documents `-f`, `-m crc=1`, size suffixes, how it ships (git tags,
  no service or config, stormblock as consumer), `write_zeroes` and when to
  override it, `Params` fields with no builder, reproducible formats through
  `uuid`/`time`/`gen_seed`, and the 12-byte label limit; the crate
  description no longer promises a checker (#7). Several rechecks of README,
  CLAUDE.md and crate docs against the code (2026-09-27, 2026-09-28).

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
