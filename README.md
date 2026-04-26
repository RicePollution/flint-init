# flint-init

A parallel-first init system for Linux. Replaces OpenRC/SysV on desktops and servers
with a dependency-DAG supervisor that starts each service the instant its dependencies
are satisfied — not in waves.

Licensed under the [GNU General Public License v2.0](LICENSE).

---

## The Problem with Wave-Based Init

OpenRC and similar systems divide services into runlevels or waves. Every service in a
wave must finish before the next wave begins. This means a fast service (dbus, 80ms)
sits idle waiting for the slowest service in its wave to complete before any of its
dependents can start.

```
OpenRC wave model:
  Wave 1: [udev .............. 800ms] [syslog .... 200ms] [←waits for udev]
  Wave 2: [dbus . 80ms] [←waits for all of wave 1 to finish]
  Wave 3: [NetworkManager] [sshd] [←wait for all of wave 2]

Total: ~800ms (wave 1) + 80ms (wave 2) + NM startup = sequential bottleneck
```

```
flint-init DAG model:
  t=0ms:    udev starts, syslog starts (no deps)
  t=200ms:  syslog signals ready (pidfile created)
  t=800ms:  udev signals ready (socket appears)
  t=800ms:  dbus starts immediately (dep satisfied)
  t=880ms:  dbus signals ready (socket accepting connections)
  t=880ms:  NetworkManager + sshd start immediately

Total: driven by the critical path, not the slowest sibling in each wave
```

On a DAG with parallel independent services, flint-init eliminates all wave-boundary
delays. The only mandatory wait is along the actual dependency chain.

---

## How It Works

### 1. Dependency DAG

Service definitions are TOML files in `/etc/flint/services/`. At startup flint-init
reads them all, builds a directed acyclic graph, and computes per-service in-degree
counts. Services with in-degree zero (no unsatisfied dependencies) are started
immediately.

```toml
[service]
name = "networkmanager"
exec = "/usr/bin/NetworkManager"
restart = "on-failure"

[deps]
needs = ["dbus"]   # hard dependency — NM fails if dbus fails

[ready]
strategy = "pidfile"
path = "/run/NetworkManager/NetworkManager.pid"
```

Two dependency types:
- `needs`: hard — if the dependency permanently fails, this service is also marked failed
- `after`: ordering only — start after, but don't require the dependency to succeed

### 2. Inotify Readiness Detection

A service is ready when it says it is — not when a launcher script exits. flint-init
watches the parent directory of each readiness path via Linux inotify and reacts the
instant the file appears. Zero polling. Zero sleep loops.

Two readiness strategies:
- `pidfile`: ready when the pidfile is created
- `socket`: ready when the socket file appears **and** a single `UnixStream::connect`
  probe succeeds. Fail-open: if the probe fails, flint-init logs a warning and signals
  ready anyway so dependents are not blocked indefinitely.

When a service signals ready, its dependents' in-degree counters are decremented. Any
that reach zero join the run queue immediately.

### 3. Direct Execution

flint-init calls `fork` + `execve` directly. No shell is forked per service. Each
service process is a direct child of the supervisor with a known PID, tracked for
SIGCHLD/waitpid.

### 4. Restart Logic

Services declare their restart policy:

| Policy | Behaviour |
|--------|-----------|
| `always` | Respawn unconditionally on exit |
| `on-failure` | Respawn only on nonzero exit code |
| `never` | Do not respawn |

After 5 failed restart attempts the service is permanently marked `Failed`. Any service
that `needs` a permanently-failed service is also marked `Failed` rather than waiting
forever.

### 5. PID 1 Duties

When running as PID 1, flint-init:
- Mounts `/proc`, `/sys`, `/dev` (devtmpfs)
- Parses `/etc/fstab` and mounts remaining real filesystems (skips root, virtual types)
- After all services exit: lazy-unmounts all real filesystems, calls `sync`, then halts or reboots

### 6. Binary Config Cache

At boot, flint-init loads service definitions from a bincode manifest
(`/etc/flint/services/services.bin`) rather than parsing TOML files directly. A
companion daemon, `flint-watch`, uses inotify to keep the manifest current whenever
a `.toml` file is added, changed, or removed — so the cold-parse cost is paid at edit
time, not at boot time.

| Boot | Config load | What happened |
|------|-------------|---------------|
| Cold (no manifest) | **114 ms** | Parsed 32 TOMLs, wrote manifest |
| Warm (manifest on disk) | **23 ms** | Read one binary file |

**4.9× faster** on every boot after the first. The 23 ms floor is kernel and filesystem
startup overhead — config parsing contributes zero.

`flint-watch` is itself a supervised service (`restart = "always"`, no dependencies)
so it is running before any other service sees a config change. The feature is
self-contained in `src/cache.rs` and `src/bin/flint_watch.rs` and can be removed with
a three-line revert if needed.

### 7. flint-ctl

A control socket at `/run/flint/ctl.sock` exposes live state via line-oriented JSON.

```
$ flint-ctl status
{
  "services": [
    { "name": "dbus",           "state": "ready",   "pid": 75  },
    { "name": "networkmanager", "state": "running",  "pid": 82  },
    { "name": "sshd",           "state": "running",  "pid": 91  },
    { "name": "syslog-ng",      "state": "running",  "pid": 68  },
    { "name": "agetty",         "state": "running",  "pid": 103 }
  ]
}

$ flint-ctl stop networkmanager
{ "ok": true }
```

---

## Boot Time Comparison

Both systems measured on the same host (i7-12700K) in QEMU with virtio networking,
512 MB RAM, 2 vCPUs. Times are from QEMU launch. The **userspace time** column isolates
the init system — it is measured from the moment PID 1 is exec'd to the moment a login
prompt appears on the serial console.

| System | Kernel | Kernel boot | Userspace (PID 1 → login) | Total |
|--------|--------|-------------|---------------------------|-------|
| OpenRC 0.63 (Alpine 3.23 live ISO) | 6.18 virt | 10,955 ms | **18,072 ms** | 29,027 ms |
| systemd 260 (Arch 2026.04 live ISO) | 6.19.10 arch | 43,857 ms | **77,913 ms** | 121,771 ms |
| flint-init (Artix, 31 services, disk) | 6.19 full | 7,419 ms | **637 ms** | 8,056 ms |

**flint-init reaches a login prompt 28× faster than OpenRC and 122× faster than systemd
from PID 1 exec.** Both ISO baselines carry live-medium overhead not present on installed
systems: Alpine squashfs verification adds ~3–5 s to OpenRC userspace; systemd's 43 s
"kernel boot" is almost entirely the 226 MB Arch initramfs decompressing before PID 1
is even exec'd, and the 78 s userspace includes one-time live-ISO tasks (rebuild linker
cache, generate SSH host keys) that don't repeat on real hardware. On an installed desktop
OpenRC typically reaches login in 12–15 s and systemd in 2–5 s; flint-init's 637 ms
includes all 31 services beginning to start.

### What OpenRC is doing for those 18 seconds

The entire init sequence runs serially. Each line below must finish before the next
begins — even though most are completely independent of each other:

```
[OpenRC 0.63 starts — t=0 relative]
 * Caching service dependencies ...         [ ok ]   │
 * Remounting devtmpfs on /dev ...          [ ok ]   │
 * Mounting /dev/mqueue ...                 [ ok ]   │
 * Mounting modloop (squashfs verify) ...   [ ok ]   │  one at a time,
 * Mounting security filesystem ...         [ ok ]   │  every step blocks
 * Mounting debug filesystem ...            [ ok ]   │  the next
 * Starting busybox mdev ...                [ ok ]   │
 * Scanning hardware for mdev ...           [ ok ]   │
 * Loading hardware drivers ...             [ ok ]   │
 * Loading modules ...                      [ ok ]   │
 * Setting system clock ...                 [ ok ]   │
 * Checking local filesystems ...           [ ok ]   │
 * Remounting filesystems ...               [ ok ]   │
 * Mounting local filesystems ...           [ ok ]   │
 * Configuring kernel parameters ...        [ ok ]   │
 * Creating user login records ...          [ ok ]   │
 * Cleaning /tmp directory ...              [ ok ]   │
 * Setting hostname ...                     [ ok ]   │
 * Starting busybox syslog ...              [ ok ]   │
 * Starting firstboot ...                   [ ok ]   │
                                                     ▼
[login prompt — t=18,072 ms]
```

### What systemd is doing for those 77,913 ms

The 43 s kernel boot is the archiso initramfs (226 MB) being decompressed and the
squashfs live root being mounted before PID 1 is exec'd. Systemd then runs in parallel
where it can, but a chain of mandatory targets still sequences startup:

```
[systemd[1] exec — t=0 relative]
  Remount root and kernel filesystems ...     [ ok ]   │
  Coldplug all udev devices ...               [ ok ]   │
  Load kernel modules ...                     [ ok ]   │  each target must be
  Rebuild dynamic linker cache ...            [ ok ]   │  reached before the
  System Initialization ...                   [ ok ]   │  next begins
  D-Bus System Message Bus ...                [ ok ]   │
  Basic System ...                            [ ok ]   │
  Generate SSH host keys ...                  [ ok ]   │
  Network ...                                 [ ok ]   │
  Permit User Sessions ...                    [ ok ]   │
  Serial Getty on ttyS0 ...                   [ ok ]   │
                                                       ▼
[archiso login: — t=77,913 ms]
```

> **Note:** "Rebuild dynamic linker cache" and "Generate SSH host keys" are live-ISO
> one-time tasks; they do not run on every boot of an installed system.

### What flint-init is doing for those 637 ms

```
[PID 1 exec — t=0]
  ├─ udev         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (socket ready at ~1500ms)
  ├─ dbus         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (socket ready at ~1800ms)
  ├─ syslog-ng    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ├─ sysctl       ━━ done                        ← no deps, runs in parallel
  ├─ hwclock      ━ done                         ← no deps, runs in parallel
  ├─ agetty       ━ LOGIN ◄── t=637ms            ← no deps, shows prompt immediately
  └─ agetty×6     ━ (tty1–tty6, also at t=0)

[udev ready → udev-trigger starts]
[dbus ready → sshd, NetworkManager, polkit start in parallel]
```

The login prompt is not held hostage by modules, clocks, or filesystems. agetty has no
dependencies in the service graph, so it starts at t=0 and the user sees a prompt in
under a second while everything else initialises in the background.

> **Measurement method:** all VMs started with `date +%s%N`, serial output captured
> to a file, `grep` polled until target strings appeared (`scripts/measure-systemd-boot.sh`
> for the Arch/systemd run). Kernel boot times differ because Alpine uses a stripped
> virt kernel, Arch uses a full desktop kernel with a 226 MB live initramfs, and Artix
> boots directly from a disk image with no initramfs at all. Both live-ISO baselines
> include overhead not present on installed systems.

---

## Real Boot — v0.4.4

flint-init booted a real Artix Linux QEMU disk image as PID 1, replacing OpenRC,
managing 31 services from `/etc/flint/services/`:

```
[pid1] mounted /proc, /sys, /dev
[flint] loaded 31 service(s)
[ctl] listening on /run/flint/ctl.sock
[flint] starting: udev
[flint] starting: syslog-ng
[flint] starting: sysctl
[flint] starting: hwclock
[flint] starting: kmod
[flint] starting: dbus
[flint] starting: agetty-tty1
[flint] starting: agetty-tty2
[flint] starting: agetty-tty3
[flint] starting: agetty-tty4
[flint] starting: agetty-tty5
[flint] starting: agetty-tty6
[flint] starting: agetty         (ttyS0)
[flint] exited: sysctl (code=0)
[flint] exited: hwclock (code=0)
[flint] exited: kmod (code=0)

Artix Linux 6.19.11-artix1-1 (ttyS0)

artixlinux login: root (automatic login)

[ready] udev ready (socket): "/run/udev/control"
[flint] starting: udev-trigger
[ready] dbus socket accepting connections: "/run/dbus/system_bus_socket"
[ready] dbus ready (socket): "/run/dbus/system_bus_socket"
[flint] starting: networkmanager
[flint] starting: sshd
[flint] starting: polkit
[flint] starting: cronie
[flint] starting: acpid
[flint] starting: irqbalance
Server listening on :: port 22.
Server listening on 0.0.0.0 port 22.
```

The login prompt appears before udev has even finished initialising. 13 services start
at t=0 (no unsatisfied dependencies): udev, syslog-ng, sysctl, hwclock, kmod, dbus,
and all seven agetty instances. sshd, NetworkManager, polkit, and others unblock the
instant dbus signals ready — they don't wait for each other or for udev.

Optional services (pipewire, bluez, cups, avahi, chronyd, smartd, wpa_supplicant,
wireplumber) are configured `restart = "never"` so a missing binary on a minimal VM
fails quietly without respawn attempts.

---

## Stage History

| Stage | Description | Version | Status |
|-------|-------------|---------|--------|
| 1 | DAG executor, inotify readiness, parallel service spawning, 41 tests | v0.1.x | Complete |
| 2 | PID 1 handoff, real service definitions, QEMU initramfs boot | v0.2.x | Complete |
| 3 | Service restart logic, socket connection probing, flint-ctl | v0.3.x | Complete |
| 4 | fstab mounts, graceful unmount, real Artix disk image boot | v0.4.x | Complete |
| 5 | Binary config cache, flint-watch inotify daemon, 4.9× faster warm boot | v0.4.17 | Complete |

---

## Building

```bash
cargo build --release
cargo test
```

Requirements: Rust 1.75+, Linux (uses inotify, fork/exec, nix syscalls).

## Running the Test Suite

```bash
cargo test   # 50 unit + integration tests, no root required
```

## QEMU Tests

```bash
# Full Artix disk image — 32 services, flint-init as PID 1 (primary test):
sudo bash scripts/create-artix-vm.sh     # one-time setup, ~5 min
sudo bash scripts/install-flint-into-vm.sh  # install/update binaries
bash scripts/boot-artix-vm.sh            # boots and tails serial log

# Initramfs boot — udev, dbus, NetworkManager (lighter, no disk image):
bash scripts/qemu-test.sh

# Stage 3 integration — restart logic + flint-ctl:
bash scripts/qemu-stage3-test.sh
```

## Service Definition Reference

```toml
[service]
name = "example"
exec = "/usr/bin/example --foreground"
restart = "on-failure"   # "always" | "on-failure" | "never"

[deps]
after = ["syslog-ng"]    # start after, don't require success
needs = ["dbus"]         # hard dep — fail if dbus fails

[ready]
strategy = "pidfile"     # "pidfile" | "socket"
path = "/run/example.pid"

[resources]
oom_score_adj = -100
```

---

## License

Copyright (C) 2026 RicePollution

This program is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software Foundation,
either version 2 of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with this
program. If not, see <https://www.gnu.org/licenses/>.
