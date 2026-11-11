# Spike 06 — Card detection (Linux half)

**Question:** How does Mirrorless notice that a camera card was inserted or removed, without polling the filesystem? This is the Linux `VolumeWatcher`; the macOS one is a Mac-session item.

**Status: watcher built, live mount test pending.** This agent's session isn't allowed to mount: polkit only allows the active desktop session, and Ubuntu's AppArmor rule blocks unprivileged mount namespaces. The test needs one manual run (below).

Code: `spikes/card-detect/` · test: `spikes/card-detect/loop-card-test.sh`

## Approach

- **Watch the mount table, not a directory.** The kernel marks `/proc/self/mountinfo` with `POLLPRI` on every mount change. The watcher `poll()`s it, re-reads it, and diffs it.
- A new mount point that contains `DCIM/` is a card. A mount point that disappears is an unmount or eject.
- **Why not inotify on `/media/$USER`** (the roadmap's first idea): udisks creates the mount-point directory *before* mounting, so a directory watcher sees an empty folder and has to retry. The mount table changes only when the filesystem is actually there. It also covers cards mounted anywhere (fstab, `/run/media`, manual mounts).
- For each new mount, the watcher logs the backing disk's sysfs `removable` flag, to check whether "removable" can be used as a filter. Built-in SD slots (`mmcblk`) often report 0, so the expectation is that **`DCIM/` is the test, not `removable`**.
- On a card, it walks `DCIM/**` (metadata only) and reports the walk time. That's step 1 of the import flow (§4.2).

## To run

```sh
cd spikes/card-detect && cargo build --release
# From a terminal in the desktop session (or with sudo):
./loop-card-test.sh            # uses /tmp/mm-card.img
```

`/tmp/mm-card.img` is a 1 GB FAT32 image labelled `EOS_DIGITAL`, with `DCIM/100CANON` and `DCIM/100NIKON` holding the 8 spike RAWs. It was built with `mkfs.vfat` + `mtools` and needs no root. The script runs the watcher, attaches the image with `udisksctl loop-setup`, mounts it if no automounter does, then unmounts and detaches it and prints the log.

**Better still:** run `target/release/card-detect` and insert a real SD card into a real reader. That also covers a built-in vs USB reader (`removable`) and pulling the card without ejecting.

## Results

*Pending the run above.*

## Expected decisions

1. Linux `VolumeWatcher` = `mountinfo` `POLLPRI` + `DCIM/` check. No `notify` crate needed.
2. **macOS:** the same race exists for an FSEvents watch on `/Volumes`. Prefer DiskArbitration (`DARegisterDiskAppearedCallback` / `DiskDisappeared` with a mounted-volume path) and keep `DCIM/` as the card test. Add this to the Mac session.
3. Surprise removal during import must surface as an I/O error on the import job group (§3.4), never as a panic. Test it with a real card on both platforms.
