# mkfs-xfs

Async **XFS** formatter and checker in pure Rust — the XFS sibling of
[mkfs.ext4.rs](https://github.com/glennswest/mkfs.ext4.rs): a from-scratch
reimplementation of `mkfs.xfs` and `xfs_repair -n`, written from the XFS
on-disk format specification and then **held to real `mkfs.xfs` output** — a
comparison tool reports every difference between the two filesystems, and the
golden tests fail on any.

Why: stormcos formats large volumes (PB-scale pools) and imports images whose
root filesystem is XFS (RHEL, Rocky, Alma). Like the ext4 crate it writes
through an async block interface — no kernel, no mount, no loop device, no root.

## Status

Scaffold (2026-09-25). The plan is in CLAUDE.md.
