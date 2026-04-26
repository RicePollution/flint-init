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
- `socket`: ready when the socket file appears **and** a connection probe succeeds
  (loops `UnixStream::connect` for up to 5 seconds to confirm the daemon is accepting)

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

### 6. flint-ctl

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
| flint-init (Artix, 31 services, disk) | 6.19 full | 7,419 ms | **637 ms** | 8,056 ms |

**flint-init reaches a login prompt 28× faster from PID 1 exec.** The Alpine ISO adds
overhead from squashfs verification that a real installed system avoids; without it
OpenRC userspace is roughly 12–15 s on a typical desktop, which matches user-reported
figures. flint-init's 637 ms includes all 31 services beginning to start.

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

> **Measurement method:** both VMs started with `date +%s%N`, serial output captured
> to a file, `grep` polled until target strings appeared. Kernel boot times differ
> because Alpine uses a stripped virt kernel while Artix uses a full desktop kernel.
> Alpine figures include live-ISO squashfs overhead (~3–5 s) not present on installed
> systems; real-hardware OpenRC typically runs 12–15 s to login on a desktop with
> NetworkManager and dbus.

---

## Real Boot — v0.4.1

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
[flint] starting: dbus
[flint] starting: agetty-tty1  (+ tty2 through tty6)
[flint] starting: agetty        (ttyS0)
[flint] exited: sysctl (code=0)
[flint] exited: hwclock (code=0)

Artix Linux 6.19.11-artix1-1 (ttyS0)

artixlinux login: root (automatic login)

[ready] udev ready (socket): "/run/udev/control"
[flint] starting: udev-trigger
[ready] dbus socket accepting connections: "/run/dbus/system_bus_socket"
[ready] dbus ready (socket): "/run/dbus/system_bus_socket"
[flint] starting: networkmanager
[flint] starting: sshd
[flint] starting: polkit
Server listening on :: port 22.
Server listening on 0.0.0.0 port 22.
```

The login prompt appears before udev has even finished initialising. agetty, sysctl,
hwclock, dbus, and syslog-ng all start at t=0 because they have no unsatisfied
dependencies. sshd and NetworkManager unblock the instant dbus signals ready — they
don't wait for each other or for udev.

---

## Stage History

| Stage | Description | Version | Status |
|-------|-------------|---------|--------|
| 1 | DAG executor, inotify readiness, parallel service spawning, 41 tests | v0.1.x | Complete |
| 2 | PID 1 handoff, real service definitions, QEMU initramfs boot | v0.2.x | Complete |
| 3 | Service restart logic, socket connection probing, flint-ctl | v0.3.x | Complete |
| 4 | fstab mounts, graceful unmount, real Artix disk image boot | v0.4.x | Complete |

---

## Building

```bash
cargo build --release
cargo test
```

Requirements: Rust 1.75+, Linux (uses inotify, fork/exec, nix syscalls).

## Running the Test Suite

```bash
cargo test   # 41 unit + integration tests, no root required
```

## QEMU Tests

```bash
# Initramfs boot — udev, dbus, nm-priv-helper, NetworkManager:
bash scripts/qemu-test.sh

# Stage 3 integration — restart logic + flint-ctl:
bash scripts/qemu-stage3-test.sh

# Stage 4 — full Artix disk image (requires creating the image first):
sudo bash scripts/create-artix-vm.sh     # one-time, ~5 min
bash scripts/boot-artix-vm.sh            # boots and tails serial log
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
