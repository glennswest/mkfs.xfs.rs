#!/bin/bash
# kernel-mount.sh — put our filesystems in front of a real Linux kernel.
#
# The golden and live tests prove we write what mkfs.xfs writes and that
# xfs_repair accepts it. This proves the kernel does: each image is
# formatted by mkfs-xfs, checked with xfs_repair -n, then mounted
# read-write by a real kernel in a VM, written to (a file, a directory,
# 4 MiB of data), unmounted, and checked with xfs_repair -n again. The
# second check is the one that counts: a filesystem the kernel mounts,
# writes to and leaves consistent is a working filesystem.
#
# No root: the VM boots the host's own kernel under qemu (KVM when
# /dev/kvm is usable) from an initramfs built here — busybox, xfs.ko and
# an init script — with the image as a virtio disk. Every case runs twice,
# once on our image and once on real mkfs.xfs's, so a failure on both is
# the environment's and a failure on ours alone is ours.
#
#   sc-build tests/kernel-mount.sh        (on dev.g8.lo)
#   tests/kernel-mount.sh                 (anywhere with the tools)

set -uo pipefail

FAILURES=0
ok()  { echo "  OK: $1"; }
bad() { echo "  FAIL: $1"; FAILURES=$((FAILURES+1)); }
hdr() { echo; echo "-- $1 --"; }
need() { command -v "$1" >/dev/null || { echo "missing $1 — cannot run"; exit 2; }; }

for t in qemu-system-x86_64 busybox xz python3 mkfs.xfs xfs_repair cargo; do need "$t"; done

KVER=$(uname -r)
KERNEL=""
for k in "/lib/modules/$KVER/vmlinuz" "/boot/vmlinuz-$KVER"; do
    [ -r "$k" ] && { KERNEL=$k; break; }
done
[ -n "$KERNEL" ] || { echo "no readable kernel image for $KVER"; exit 2; }
XFS_KO=$(modinfo -n xfs 2>/dev/null)
[ -n "$XFS_KO" ] || { echo "no xfs module for $KVER (built in?)"; }

ACCEL="-accel tcg"
[ -r /dev/kvm ] && [ -w /dev/kvm ] && ACCEL="-accel kvm -cpu host"

# Scratch under the target directory: per project, never a shared /tmp.
SCRATCH="${CARGO_TARGET_DIR:-$PWD/target}"
mkdir -p "$SCRATCH"
WORK=$(mktemp -d "$SCRATCH/kernel-mount.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
UUID=12345678-1234-5678-9abc-123456789abc

hdr "building mkfs-xfs"
cargo build --quiet --bin mkfs-xfs || { echo "cargo build failed"; exit 1; }
MKFS="${CARGO_TARGET_DIR:-target}/debug/mkfs-xfs"

hdr "building the initramfs (kernel $KVER, $ACCEL)"
ROOT="$WORK/root"
mkdir -p "$ROOT"/{bin,dev,proc,sys,mnt,tmp,lib64,lib}
cp "$(command -v busybox)" "$ROOT/bin/busybox"
# A dynamically linked busybox brings its libraries.
if ldd "$ROOT/bin/busybox" >/dev/null 2>&1; then
    for lib in $(ldd "$ROOT/bin/busybox" | grep -o '/[^ ]*'); do
        mkdir -p "$ROOT$(dirname "$lib")"; cp -L "$lib" "$ROOT$lib"
    done
fi
if [ -n "$XFS_KO" ]; then
    case "$XFS_KO" in
        *.xz)  xz -dc "$XFS_KO" > "$ROOT/xfs.ko" ;;
        *.zst) zstd -qdc "$XFS_KO" > "$ROOT/xfs.ko" ;;
        *)     cp "$XFS_KO" "$ROOT/xfs.ko" ;;
    esac
fi
cat > "$ROOT/init" <<'INIT'
#!/bin/busybox sh
/bin/busybox mount -t proc proc /proc
/bin/busybox --install -s /bin
export PATH=/bin
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
[ -f /xfs.ko ] && insmod /xfs.ko
r() { echo "RESULT $1"; }
if mount -t xfs -o rw /dev/vda /mnt 2>/tmp/err; then
    r MOUNT_OK
    grep ' /mnt ' /proc/mounts | grep -q ' rw' && r RW_OK || r RW_FAIL
    if echo "hello from the kernel" > /mnt/probe.txt && [ "$(cat /mnt/probe.txt)" = "hello from the kernel" ]; then
        r WRITE_OK
    else
        r WRITE_FAIL
    fi
    mkdir -p /mnt/adir/sub && echo x > /mnt/adir/sub/f && r MKDIR_OK || r MKDIR_FAIL
    # Enough files to leave the short-form root directory and allocate
    # new inode chunks beyond the one mkfs made.
    i=0; while [ $i -lt 300 ]; do echo $i > /mnt/adir/file$i || break; i=$((i+1)); done
    [ $i -eq 300 ] && r MANYFILES_OK || r MANYFILES_FAIL
    dd if=/dev/urandom of=/mnt/big.bin bs=1M count=4 2>/dev/null && sync && r BIGWRITE_OK || r BIGWRITE_FAIL
    echo "STATFS $(stat -f -c '%b %f %c %d' /mnt)"
    umount /mnt && r UMOUNT_OK || r UMOUNT_FAIL
    # And back: the kernel reads what it wrote, from a clean log.
    if mount -t xfs -o ro /dev/vda /mnt 2>>/tmp/err; then
        [ "$(cat /mnt/probe.txt)" = "hello from the kernel" ] && [ -f /mnt/adir/file299 ] \
            && r REMOUNT_OK || r REMOUNT_FAIL
        umount /mnt
    else
        r REMOUNT_FAIL
    fi
else
    r MOUNT_FAIL
fi
sed 's/^/ERR /' /tmp/err
dmesg | grep -i xfs | sed 's/^/DMESG /'
echo "RESULT DONE"
poweroff -f
INIT
chmod +x "$ROOT/init"

# An initramfs with a /dev/console node, written without mknod (no root):
# newc headers are only bytes.
python3 - "$ROOT" "$WORK/initrd.cpio" <<'PY'
import os, stat, sys
root, out = sys.argv[1], sys.argv[2]
ino = [1]
def hdr(name, mode, size, nlink=1, rmaj=0, rmin=0):
    ino[0] += 1
    fields = [ino[0], mode, 0, 0, nlink, 0, size, 0, 0, rmaj, rmin, len(name) + 1, 0]
    h = b"070701" + b"".join(b"%08x" % f for f in fields)
    h += name.encode() + b"\0"
    return h + b"\0" * ((4 - len(h) % 4) % 4)
def pad(d):
    return d + b"\0" * ((4 - len(d) % 4) % 4)
with open(out, "wb") as f:
    for dirpath, dirs, files in os.walk(root):
        rel = os.path.relpath(dirpath, root)
        if rel != ".":
            f.write(hdr(rel, stat.S_IFDIR | 0o755, 0, 2))
        for n in files:
            p = os.path.join(dirpath, n)
            r = os.path.normpath(os.path.join(rel, n))
            st = os.lstat(p)
            if stat.S_ISLNK(st.st_mode):
                t = os.readlink(p).encode()
                f.write(hdr(r, stat.S_IFLNK | 0o777, len(t)) + pad(t))
            else:
                d = open(p, "rb").read()
                f.write(hdr(r, stat.S_IFREG | (st.st_mode & 0o777), len(d)) + pad(d))
    f.write(hdr("dev/console", stat.S_IFCHR | 0o600, 0, 1, 5, 1))
    f.write(hdr("TRAILER!!!", 0, 0))
PY
ok "initramfs $(du -h "$WORK/initrd.cpio" | cut -f1)"

# name : size : mkfs options (the same for mkfs-xfs and mkfs.xfs)
CASES=(
    "512m-default:512M:"
    "1g-b1024:1G:-b size=1024"
    "2g-s4096:2G:-s size=4096"
    "1g-agcount7-label:1G:-d agcount=7 -L kernel"
    "2g-b2048-i1024:2G:-b size=2048 -i size=1024"
    "512m-v5-minimal:512M:-m finobt=0,rmapbt=0,reflink=0,inobtcount=0 -i sparse=0,nrext64=0"
    "16g-default:16G:"
    "2g-b16384:2G:-b size=16384"
)

boot() { # image -> VM output
    timeout 300 qemu-system-x86_64 $ACCEL -m 1024 -smp 2 -nographic -no-reboot \
        -kernel "$KERNEL" -initrd "$WORK/initrd.cpio" \
        -append "console=ttyS0 panic=-1 loglevel=4 rdinit=/init" \
        -drive "file=$1,format=raw,if=virtio,cache=unsafe" 2>&1
}

repair() { xfs_repair -n -f "$1" >"$1.repair" 2>&1; }

CHECKS="MOUNT_OK RW_OK WRITE_OK MKDIR_OK MANYFILES_OK BIGWRITE_OK UMOUNT_OK REMOUNT_OK"

for case in "${CASES[@]}"; do
    IFS=: read -r name size opts <<< "$case"
    for who in ours theirs; do
        img="$WORK/$name.$who.img"
        rm -f "$img"; truncate -s "$size" "$img"
        # shellcheck disable=SC2086
        if [ $who = ours ]; then
            "$MKFS" -q -m uuid=$UUID $opts "$img" || { bad "$name/$who: mkfs-xfs"; continue; }
        else
            mkfs.xfs -q -f -m uuid=$UUID $opts "$img" || { bad "$name/$who: mkfs.xfs"; continue; }
        fi
    done
    echo
    echo "  == $name ($size $opts) =="
    for who in ours theirs; do
        img="$WORK/$name.$who.img"
        [ -f "$img" ] || continue
        repair "$img" && pre=OK || pre=FAIL
        out=$(boot "$img")
        repair "$img" && post=OK || post=FAIL
        results=""
        for c in $CHECKS; do
            grep -q "^RESULT $c" <<< "$out" && results="$results ${c%_OK}" || results="$results !${c%_OK}"
        done
        if [ $who = ours ]; then
            [ $pre = OK ] && ok "xfs_repair -n before" || { bad "xfs_repair -n before"; cat "$img.repair" | head -20; }
            for c in $CHECKS; do
                grep -q "^RESULT $c" <<< "$out" && ok "kernel: ${c%_OK}" || bad "kernel: ${c%_OK}"
            done
            [ $post = OK ] && ok "xfs_repair -n after the kernel wrote" || { bad "xfs_repair -n after"; head -20 "$img.repair"; }
            grep -q '^RESULT DONE' <<< "$out" || { echo "    (VM did not finish)"; tail -30 <<< "$out" | sed 's/^/    /'; }
            grep -E '^(ERR|STATFS)' <<< "$out" | sed 's/^/    /'
            grep '^DMESG' <<< "$out" | grep -iE 'error|corrupt|fail|warn' | sed 's/^/    /' | head -10
        else
            echo "    control (real mkfs.xfs): repair-before=$pre repair-after=$post kernel:$results"
        fi
    done
    rm -f "$WORK/$name".*.img
done

hdr "result"
if [ "$FAILURES" -eq 0 ]; then
    echo "all checks passed"
    exit 0
fi
echo "$FAILURES check(s) failed"
exit 1
