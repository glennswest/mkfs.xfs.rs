#!/usr/bin/env python3
"""Capture golden reference filesystems from real mkfs.xfs.

    tests/golden/capture.py            # every case in CASES
    tests/golden/capture.py NAME ...   # just these

Each case is formatted by the mkfs.xfs on PATH into a sparse scratch file
with the UUID pinned, and stored as NAME.sparse.gz: only the non-zero
blocks, so a 1 GiB filesystem is a few kilobytes. NAME.args records the
exact command line and the mkfs.xfs version. tests/golden.rs formats the
same size and options with this crate and requires no structural
difference.

Format of the .sparse stream (gzip compressed):
    b"XFSSPRS1", u64 BE image size, then records of
    u64 BE offset, u32 BE length, that many bytes.
"""
import gzip, os, struct, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
UUID = "12345678-1234-5678-9abc-123456789abc"
BLOCK = 4096

# name: (size, mkfs.xfs options). Mirror them in tests/golden.rs.
CASES = {
    "xfs-512m-default": ("512M", []),
    "xfs-300m-default": ("300M", []),
    "xfs-1g-b1024": ("1G", ["-b", "size=1024"]),
    "xfs-2g-b2048-i1024": ("2G", ["-b", "size=2048", "-i", "size=1024"]),
    "xfs-2g-s4096": ("2G", ["-s", "size=4096"]),
    "xfs-1g-agcount7-label": ("1G", ["-d", "agcount=7", "-L", "golden"]),
    "xfs-600m-agcount2": ("600M", ["-d", "agcount=2"]),
    "xfs-512m-v5-minimal": ("512M", ["-m", "finobt=0,rmapbt=0,reflink=0,inobtcount=0",
                                     "-i", "sparse=0,nrext64=0"]),
    "xfs-1g-b16384": ("1G", ["-b", "size=16384"]),
    "xfs-2g-s4096-b16384": ("2G", ["-s", "size=4096", "-b", "size=16384"]),
    # Past what the build box's filesystem holds as a file: captured where
    # a 1 PiB sparse file is possible.
    "xfs-16t-default": ("16T", []),
    "xfs-1p-default": ("1P", []),
}


def data_extents(fd, total):
    """(start, end) of each allocated extent: a 1 PiB image is read only
    where it holds data."""
    off = 0
    while off < total:
        try:
            start = os.lseek(fd, off, os.SEEK_DATA)
        except OSError:
            return
        end = os.lseek(fd, start, os.SEEK_HOLE)
        yield start, end
        off = end


def nonzero_runs(path, total):
    """Runs of non-zero blocks, as (offset, bytes)."""
    with open(path, "rb") as f:
        for start, end in data_extents(f.fileno(), total):
            start -= start % BLOCK
            f.seek(start)
            run_start, run = None, []
            off = start
            while off < end:
                blk = f.read(min(BLOCK, total - off))
                if not blk:
                    break
                if any(blk):
                    if run_start is None:
                        run_start = off
                    run.append(blk)
                elif run_start is not None:
                    yield run_start, b"".join(run)
                    run_start, run = None, []
                off += len(blk)
            if run_start is not None:
                yield run_start, b"".join(run)


def capture(name, size, opts):
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as tmp:
        img = os.path.join(tmp, "img")
        subprocess.run(["truncate", "-s", size, img], check=True)
        cmd = ["mkfs.xfs", "-q", "-f", "-m", f"uuid={UUID}", *opts, img]
        subprocess.run(cmd, check=True)
        version = subprocess.run(["mkfs.xfs", "-V"], capture_output=True, text=True).stdout.strip()
        total = os.path.getsize(img)
        out = [b"XFSSPRS1", struct.pack(">Q", total)]
        for start, data in nonzero_runs(img, total):
            out.append(struct.pack(">QI", start, len(data)) + data)
    with open(os.path.join(HERE, f"{name}.sparse.gz"), "wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=9, mtime=0) as g:
            g.write(b"".join(out))
    with open(os.path.join(HERE, f"{name}.args"), "w") as a:
        a.write(f"# {version}\n# truncate -s {size} img\n")
        a.write(" ".join(c if c != img else "img" for c in cmd) + "\n")
    print(f"{name}: {size} {' '.join(opts)}")


def main():
    names = sys.argv[1:] or list(CASES)
    for n in names:
        capture(n, *CASES[n])


if __name__ == "__main__":
    main()
