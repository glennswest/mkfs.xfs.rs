# CLAUDE.md — mkfs.xfs.rs

XFS formatter and `xfs_admin -U/-L` (checker planned, #7) in pure Rust.
Follow mkfs.ext4.rs's structure and
rules (read its CLAUDE.md): from the on-disk spec, held to real `mkfs.xfs`
output by a comparison tool and golden tests, async block I/O, no kernel.

- **Crate:** `mkfs-xfs` (lib `mkfs_xfs`), binaries `mkfs-xfs` and
  `xfs-admin` (feature `cli`, on by default)
- **Ships as:** git tags; no service, ports or config file; not a
  stormcentral component (no golden). **Consumer:** stormblock
  (`tag = "v0.2.0"`, `default-features = false`, `src/fs/xfs.rs`) formats
  XFS volumes and stamps clone UUIDs. Keep public and stable everything it
  uses: `device::BlockDevice` (it implements `size`, `logical_sector_size`,
  `read_at`, `write_at`, `flush`, `write_zeroes`), `Error::io`, `Result`,
  `geometry::Params::new()` with `.uuid()`/`.label()`/`.block_size()`,
  `format::format` and `Report` (`geometry.{blocksize,dblocks,agcount,
  agsize,logblocks}`, `fdblocks`, `uuid`), `crc::verify`/`crc::stamp`, and
  `structs::sb::off`/`version`. `admin::set_uuid`/`set_label` (#6) are
  there for it to replace its own superblock stamping.
- **Version:** see `Cargo.toml` and `VERSION` (both must match)
- **Reference:** xfsprogs 6.15.0 (`mkfs/xfs_mkfs.c`, `libxfs/xfs_ag.c`,
  `libxfs/topology.c`, `libxfs/rdwr.c`, `mkfs/proto.c`) — the version on
  dev.g8.lo, and the one every golden was captured with. Consult it at the
  point where a difference shows up.

## Layout

| Module | What it owns |
|---|---|
| `device` | async `BlockDevice` (`&self` reads and writes); `FileDevice`; sparse `MemDevice` (and `strict`, which refuses sub-sector I/O) |
| `geometry` | `Params` (the mkfs.xfs command line) → `Geometry`: block/sector/inode sizes, AG count/size (`calc_default_ag_geometry`), log size and place (`calculate_log_size`), fixed per-AG block positions, root inode location |
| `structs` | byte-exact encoders with named field offsets: `sb`, `ag` (AGF/AGI/AGFL), `btree` (short-form v5 blocks and records), `inode` (v3 dinode, short-form dir), `log` (first record + unmount record) |
| `format` | `plan_ag` computes each AG's end state (free extents, AGFL, rmap records, root chunk); `format` writes it, AGs in parallel |
| `inspect` | reads any v5 XFS into named, classed fields (structural / identity / incidental) |
| `compare` | diffs two dumps |
| `admin` | `xfs_admin -U`/`-L`: log head/tail search (libxlog port), full log clear, per-AG superblock rewrite as `xfs_db` does it |

## Rules learned from the goldens

- mkfs.xfs's end state is computed, not replayed: AGFL = `min_freelist`
  blocks from the start of the smallest free extent that holds them
  (`xfs_alloc_ag_vextent_size`), slots 1..=n (`agf_flfirst` starts at 1);
  root chunk where `xfs_ialloc_calc_rootino` says; rmap records inserted
  after the static ones merge with a same-owner neighbour.
- Secondary superblocks keep the header-init state: `inprogress = 1`,
  counters 0, `fdblocks = dblocks - agcount*prealloc - logblocks`, inode
  pointers NULL — except `rootino`, rewritten in the last AG and the middle
  one (`(agcount-1)/2`, only when agcount > 2).
- `sb_rextsize` = 4 KiB in blocks (4 at 1 KiB); a log sector > 512 bytes
  gives `sb_logsunit` = one block, and the first log record spans it.
- Root inode gen is 0; every other new inode's gen is random, and a fresh
  chunk shares one random gen.

## Work plan

- [x] Issue #1 — format a valid XFS filesystem, held to real mkfs.xfs
      (2026-09-25). Superblocks, AGF/AGI/AGFL, bno/cnt/ino/fino/rmap/refc
      roots, root inode chunk (root dir, rt bitmap and summary inodes), log.
      Verified: 12 goldens (300 MB – 1 PiB) field-identical to mkfs.xfs
      6.15.0; live comparison + `xfs_repair -n` to 8 TiB on dev;
      `tests/kernel-mount.sh` mounts, writes, remounts in a VM on dev.
- [x] Geometry defaults: AG count/size, sector/block/inode sizes, log —
      matching mkfs.xfs for a file or rotational disk
- [ ] Stripe geometry (su/sw, device io_min/io_opt) — issue #4
- [ ] Minimum log size: < 300 MB and block sizes > 16 KiB — issue #2
- [ ] Non-rotational (concurrency) geometry — issue #3 (owner decision on
      whether to use it: #8)
- [x] CLI `-N`/post-format report printed log `sunit=0` where the image has
      a one-block log stripe unit (4 KiB log sectors) — issue #9, fixed
      2026-10-06 (v0.2.1); `tests/cli_report.rs` holds the log lines to
      `mkfs.xfs -N`.
- [ ] Overwrite hygiene: stale secondaries, discard — issue #5
- [ ] `xfs_admin -U/-L` over `BlockDevice` for stormblock's clones — issue #6
      (in progress 2026-10-06). `admin::set_uuid` = xfs_db `uuid_f`: refuse
      NEEDSREPAIR / realtime / external log; find the log head and tail
      (`libxlog` `xlog_find_tail`, userspace port), refuse a dirty log;
      `libxfs_log_clear` at `l_curr_cycle + 1` with the new UUID (rewrites
      the whole log, as xfs_admin does); then every AG's sb (`do_uuid`:
      META_UUID on/off). `admin::set_label` = `label_f`/`do_label`. Held to
      `xfs_admin` byte for byte in `tests/live_xfs_admin.rs`.
- [x] Kernel verification in a throwaway VM via `stormcentral testhost boot`
      — issue #11, done 2026-10-06. `tests/vm/build-image.sh OUT` (sc-build
      with `SC_BUILD_OUT`) makes a GPT disk: ESP with the UEFI Shell,
      `startup.nsh` → dev's kernel (EFI stub) + a busybox initramfs with the
      loop/xfs modules, xfs_repair and our mkfs-xfs / xfs-admin.
      `tests/vm/init.sh` formats 8 cases, loop-mounts, writes, `xfs_repair
      -n`, remounts; again after xfs-admin -U; two clones of one blank side
      by side. First pass: nanatest1 run 7931c56f7e (kernel 7.2.8, 12 s).
- [ ] Lazy formatting where XFS allows. Note: a format already writes only
      headers, roots, one inode chunk and the log (1 PiB: ~0.1 s in memory);
      what remains is the 2 GiB log zeroing on devices without write-zeroes.
- [x] Comparison tool vs real mkfs.xfs and golden tests, run on dev
- [ ] Checker (an `xfs_repair -n` subset): structural validation — build it
      on `inspect` — issue #7
- [ ] Large sizes measured on stormcos's emulated large drives (layout at
      16 TiB and 1 PiB already golden-tested)

## Testing

- `sc-build` — unit tests, `tests/golden.rs`, `tests/live_mkfs_xfs.rs`
  (uses dev's mkfs.xfs/xfs_repair; skips elsewhere), `tests/live_xfs_admin.rs`
  (byte-identical to dev's xfs_admin), `tests/device_io.rs`.
- `tests/vm/` — the same in a throwaway VM through `stormcentral testhost
  boot nanatest1` (README "How it is held"); pass line `VERIFY PASS`.
- `sc-build tests/kernel-mount.sh` — kernel mount/write/remount in qemu+KVM
  with the host kernel and an initramfs built from busybox and xfs.ko; no
  root. Runs each case on mkfs.xfs's image too, as a control.
- New goldens: add the case to `tests/golden/capture.py` and
  `tests/golden.rs`, run `capture.py NAME` where xfsprogs 6.15.0 is (this VM
  has it, and its /home takes 1 PiB sparse files; dev's build filesystem
  stops at 16 TiB).

## Conventions

- Every on-disk structure names the xfsprogs struct and function it mirrors;
  field offsets are named constants shared by encoder and reader.
- No `unsafe`. Big-endian field by field, never a repr(C) cast.
- Every device operation is whole filesystem blocks at a block boundary;
  only `inspect::geometry`'s first read is a sector.
- Refuse (`Error::Unsupported`) what cannot be reproduced exactly; never
  approximate mkfs.xfs.
