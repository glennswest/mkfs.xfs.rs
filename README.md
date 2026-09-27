# mkfs-xfs

Async **XFS** formatter in pure Rust — the XFS sibling of
[mkfs.ext4.rs](https://github.com/glennswest/mkfs.ext4.rs): a from-scratch
reimplementation of `mkfs.xfs`, written from the XFS on-disk format and
**held to real `mkfs.xfs` output** — a comparison tool reports every
difference between two filesystems, and the golden tests fail on any.

Why: stormcos formats large volumes (PB-scale pools) and imports images whose
root filesystem is XFS (RHEL, Rocky, Alma). Like the ext4 crate it writes
through an async block interface — no kernel, no mount, no loop device, no
root.

## Status

Formats a v5 XFS filesystem identical to what `mkfs.xfs` 6.15 writes for the
same size and options, from 300 MB to 1 PiB — every superblock, AGF, AGI,
AGFL, free-space / inode / free-inode / reverse-mapping / refcount btree
root, the root inode chunk and the log — differing only in what `mkfs.xfs`
draws at random or from the clock (timestamps, inode generation numbers, and
the CRCs over them). The kernel mounts it read-write, writes to it and
leaves it consistent. There is no checker yet — an `xfs_repair -n` subset
is planned (#7); see the work plan in `CLAUDE.md`.

## Use

```rust
use mkfs_xfs::{device::FileDevice, format::format, geometry::Params};

let dev = FileDevice::open("/dev/nvme1n1").await?;
let report = format(&dev, &Params::new().label("data")).await?;
```

`BlockDevice` is the seam: a consumer formats its own volume (in memory,
network-backed, a thin volume) by implementing it, and nothing else touches
a `/dev` node. `MemDevice` is sparse, so a 1 PiB filesystem formats in memory
in a fraction of a second. The trait's default `write_zeroes` writes zero
blocks, 1 MiB at a time; a device with write-zeroes or a discard that zeroes
should override it, since the log alone can be 2 GiB (`FileDevice` punches
holes in regular files).

Every `Params` field is optional and left to `mkfs.xfs`'s choice when unset
(`agsize`, `imaxpct` and the rest are public fields; the common ones also
have builder methods). `uuid`, `time` and `gen_seed` pin what `mkfs.xfs`
draws at random or from the clock, so a format is reproducible byte for
byte. A label is at most 12 bytes.

The CLI takes `mkfs.xfs`'s option syntax for what it supports:

    mkfs-xfs [-b size=N] [-s size=N] [-i size=N,maxpct=N,sparse=0|1,nrext64=0|1]
             [-d size=N,agcount=N,agsize=N] [-L label]
             [-m uuid=U,finobt=0|1,rmapbt=0|1,reflink=0|1,inobtcount=0|1,bigtime=0|1]
             [-N] [-q] [-f] [--create SIZE] DEVICE

`-N` prints the geometry and writes nothing (its log `sunit` reads 0 even
where the filesystem has a one-block log stripe unit — #9); `-f` is accepted and ignored
(an existing filesystem is never refused); `-m crc=1` is accepted, anything
else not listed is an error. `--create SIZE` makes a sparse image file of
that size first. Sizes take `k`/`m`/`g`/`t`/`p` (binary) suffixes. Install
the binary as `mkfs.xfs` if `mkfs -t xfs` should find it.

## Defaults, and what is refused

The defaults are `mkfs.xfs` 6.15's: 4 KiB blocks, 512-byte inodes, the
device's physical sector, CRCs, finobt, rmapbt, reflink, inobtcount, bigtime,
nrext64 and sparse inodes, the AG count and log size its calculations give.

- **Geometry is `mkfs.xfs`'s for a file or a rotational disk.** On a
  non-rotational device `mkfs.xfs` instead sizes AGs and the log from the
  formatting host's CPU count; that path is not reproduced (#3). Per the
  xfsprogs source this is also the geometry of `mkfs.xfs -d concurrency=0
  -l concurrency=0`; that equivalence is not tested.
- **No stripe geometry yet** (`su`/`sw`, or a device's reported `io_min` /
  `io_opt`) — #4.
- **An old filesystem on the device is not cleared** beyond the first and
  last 128 KiB: no discard, stale secondary superblocks stay — #5.
- **No `xfs_admin`-style UUID/label change** — #6.
- **Refused, not approximated:** filesystems under 300 MB (which `mkfs.xfs`
  also refuses) and block sizes over 16 KiB. Both are where `mkfs.xfs` sizes
  the log from the minimum log size, which is the whole transaction
  reservation table and not implemented here (#2).

## How it is held to `mkfs.xfs`

| Test | What it proves |
|---|---|
| `tests/golden.rs` | 12 images made by `mkfs.xfs` 6.15.0 with the UUID pinned (`tests/golden/`, stored sparse: 2 KB each, 184 KB for 1 PiB), from 300 MB to 1 PiB, across block sizes 1–16 KiB, 512 and 4096-byte sectors, inode sizes, AG counts, a label and a minimal feature set. Ours must match every structural field. |
| `tests/live_mkfs_xfs.rs` | Against the `mkfs.xfs` installed where the tests run (dev.g8.lo), 777 MB to 8 TiB: field-for-field equal, and `xfs_repair -n` clean. Skips where there is no xfsprogs. |
| `tests/kernel-mount.sh` | A real kernel, in a VM, no root: `xfs_repair -n`, mount read-write, write a file, directories, 300 files and 4 MiB, unmount, remount, `xfs_repair -n` again — on our image and, as a control, on `mkfs.xfs`'s. |
| `tests/device_io.rs` | Whole-block I/O on a device that enforces a 4 KiB sector; a 1 PiB format in memory. |

    sc-build                          # build and every Rust test
    sc-build tests/kernel-mount.sh    # the kernel test
    cargo run --example compare -- ours.img theirs.img [--all]
    cargo run --example dump -- fs.img

The comparison (`compare`) reads both filesystems with `inspect` — every
superblock, AG header, btree block (walked from its root), every inode of
every allocated chunk, the log record and whether the rest of the log is
zero — into named fields, and classes each difference: **structural**,
**identity** (the UUID) or **incidental** (random or clock-derived).
Golden images are captured with `tests/golden/capture.py`.

## How it ships

A library crate (`mkfs-xfs`, lib `mkfs_xfs`) plus the `mkfs-xfs` binary
behind the default `cli` feature. No service, ports or configuration file:
everything is `Params` (or the CLI flags above). Releases are git tags
(`v0.2.0`); consumers depend on a tag with `default-features = false`.
stormblock does, to format XFS volumes, and stamps each clone's UUID with
`structs::sb::off` / `structs::sb::version` — so those offsets and the
public `structs` API are relied on (the operation itself moving here is
#6). It is not a stormcentral component: there is no golden.

## License

MIT OR Apache-2.0
