#!/bin/busybox sh
# The init of the kernel-verification VM (#11), run as PID 1 from the
# initramfs tests/vm/build-image.sh makes. Everything here is the real
# kernel's: our mkfs-xfs formats sparse image files in tmpfs, and the kernel
# loop-mounts them, writes, unmounts and remounts; xfs_repair -n (xfsprogs,
# from the build box) checks each before and after. Then the clone case
# stormblock lives on: two copies of one blank (same UUID) cannot be mounted
# together; after xfs-admin -U on one, they can.
#
# Prints `VERIFY PASS` or `VERIFY FAIL <why>` on the serial console, then
# powers off. stormcentral's testhost boot watches for those lines.
/bin/busybox mount -t proc proc /proc
/bin/busybox --install -s /bin
export PATH=/bin:/sbin
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
mkdir -p /mnt /mnt2 /work
mount -t tmpfs -o size=90% tmpfs /work
echo 1 > /proc/sys/kernel/printk 2>/dev/null

say() { echo "XFS-VERIFY: $*"; }
fail() {
    say "FAIL: $*"
    dmesg | grep -iE 'xfs|loop' | tail -20 | sed 's/^/XFS-VERIFY dmesg: /'
    [ -s /work/repair.log ] && head -30 /work/repair.log | sed 's/^/XFS-VERIFY repair: /'
    echo "VERIFY FAIL $*"
    sync; poweroff -f; sleep 30
}

say "kernel $(uname -r), $(cat /build-info 2>/dev/null)"
for m in $(cat /modules.order 2>/dev/null); do
    insmod "/modules/$m" || fail "insmod $m"
done
grep -qw xfs /proc/filesystems || fail "the kernel has no xfs"

UUID=12345678-1234-5678-9abc-123456789abc
NEW=0badc0de-1234-4321-8765-0123456789ab

repair() { # image what
    xfs_repair -n -f "$1" >/work/repair.log 2>&1 || fail "xfs_repair -n $2"
}

exercise() { # image name — mount rw, write, unmount, check, remount ro
    img=$1; name=$2
    mount -t xfs -o loop,rw "$img" /mnt 2>/work/err || fail "$name: mount: $(cat /work/err)"
    grep -q ' /mnt xfs rw' /proc/mounts || fail "$name: not mounted read-write"
    echo "hello from the kernel" > /mnt/probe.txt || fail "$name: write"
    mkdir -p /mnt/adir/sub && echo x > /mnt/adir/sub/f || fail "$name: mkdir"
    # Out of the short-form root directory and into new inode chunks.
    i=0; while [ $i -lt 300 ]; do echo $i > /mnt/adir/file$i || break; i=$((i+1)); done
    [ $i -eq 300 ] || fail "$name: only $i of 300 files"
    dd if=/dev/urandom of=/mnt/big.bin bs=1M count=4 2>/dev/null || fail "$name: 4 MiB write"
    sum=$(md5sum /mnt/big.bin | cut -d' ' -f1)
    sync
    umount /mnt || fail "$name: umount"
    repair "$img" "$name: after the kernel wrote"
    mount -t xfs -o loop,ro "$img" /mnt 2>/work/err || fail "$name: remount: $(cat /work/err)"
    [ "$(cat /mnt/probe.txt)" = "hello from the kernel" ] && [ -f /mnt/adir/file299 ] \
        && [ "$(md5sum /mnt/big.bin | cut -d' ' -f1)" = "$sum" ] || fail "$name: read back"
    umount /mnt || fail "$name: umount after remount"
}

# name : size : mkfs-xfs options
for case in \
    "512m-default:512M:" \
    "1g-b1024:1G:-b size=1024" \
    "2g-s4096:2G:-s size=4096" \
    "1g-agcount7-label:1G:-d agcount=7 -L kernel" \
    "2g-b2048-i1024:2G:-b size=2048 -i size=1024" \
    "512m-v5-minimal:512M:-m finobt=0,rmapbt=0,reflink=0,inobtcount=0 -i sparse=0,nrext64=0" \
    "16g-default:16G:" \
    "2g-b16384:2G:-b size=16384"
do
    name=${case%%:*}; rest=${case#*:}; size=${rest%%:*}; opts=${rest#*:}
    img=/work/$name.img
    rm -f "$img"; truncate -s "$size" "$img" || fail "$name: truncate"
    # shellcheck disable=SC2086
    mkfs-xfs -q -m uuid=$UUID $opts "$img" >/work/err 2>&1 || fail "$name: mkfs-xfs: $(cat /work/err)"
    repair "$img" "$name: as formatted"
    exercise "$img" "$name"
    # A new UUID over the log the kernel left, then the kernel again.
    xfs-admin -U $NEW "$img" >/work/err 2>&1 || fail "$name: xfs-admin -U: $(cat /work/err)"
    repair "$img" "$name: after xfs-admin -U"
    exercise "$img" "$name (new UUID)"
    say "$name ($size $opts): mounted, written, remounted, xfs_repair -n clean; again after xfs-admin -U"
    rm -f "$img"
done

# Clones of one blank: the same UUID twice is refused by the kernel, and
# xfs-admin -U is what makes the second mountable.
for c in a b; do
    truncate -s 1G /work/clone-$c.img
    mkfs-xfs -q -m uuid=$UUID -L blank /work/clone-$c.img >/work/err 2>&1 || fail "clone $c: mkfs-xfs: $(cat /work/err)"
done
mount -t xfs -o loop /work/clone-a.img /mnt 2>/work/err || fail "clone a: mount: $(cat /work/err)"
if mount -t xfs -o loop /work/clone-b.img /mnt2 2>/dev/null; then
    fail "clone b mounted beside clone a with the same UUID"
fi
umount /mnt
xfs-admin -U $NEW -L clone-b /work/clone-b.img >/work/err 2>&1 || fail "clone b: xfs-admin: $(cat /work/err)"
repair /work/clone-b.img "clone b after xfs-admin"
mount -t xfs -o loop /work/clone-a.img /mnt 2>/work/err || fail "clone a: mount: $(cat /work/err)"
mount -t xfs -o loop /work/clone-b.img /mnt2 2>/work/err || fail "clone b beside a after xfs-admin -U: $(cat /work/err)"
echo a > /mnt/who && echo b > /mnt2/who || fail "clones: write"
umount /mnt2 && umount /mnt || fail "clones: umount"
repair /work/clone-a.img "clone a after use"
repair /work/clone-b.img "clone b after use"
say "clones: same UUID refused side by side; after xfs-admin -U both mount, write, xfs_repair -n clean"

echo "VERIFY PASS"
sync; poweroff -f; sleep 30
