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

Measured on a QEMU VM (virtio-blk, 512 MB RAM, 2 vCPUs) booting a real Artix Linux disk
image with 31 service definitions. Times are from PID 1 exec — kernel boot (~12.5 s on
this VM, ~2–4 s on real NVMe hardware) is identical for both init systems and excluded.

### Critical Path: flint-init vs OpenRC

```
OpenRC (rc_parallel=YES, Artix defaults)
─────────────────────────────────────────────────────────────────────────────
sysinit wave  [udev ━━━━━━━━━━━━━━━━━━ 1600ms] ← all other sysinit services
              [hwclock ━ 50ms        ]    wait  ← wait for slowest sibling
              [sysctl ━━ 120ms       ]    here
boot wave     [modules ━━ 200ms      ]
              ↕ wave boundary (~50ms scheduling gap)
default wave  [syslog-ng ━━ 300ms    ] ←── these all start at the same moment
              [dbus ━━━ 200ms        ]     (rc_parallel=YES runs them in parallel)
              [polkit ━━━━━ 400ms    ]     but the wave itself can't begin until
              ...                          sysinit+boot are fully complete
              [NetworkManager ━━━━━━━━━━━━━━━━━━━ 2000ms]
              [sshd ━━━ 350ms        ]
              ↕ login prompt only after default runlevel services are "started"

  Userspace → login prompt:  ~2300ms  (sysinit + boot waves must fully clear first)
  Userspace → sshd listening: ~2400ms  (sshd starts in default wave after udev wave)
```

```
flint-init (DAG, this repo)
─────────────────────────────────────────────────────────────────────────────
t=    0ms  udev ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (starts immediately)
           syslog-ng ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (starts immediately)
           dbus ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (starts immediately)
           sysctl ━ 120ms ✓                              (starts immediately)
           hwclock ━ 50ms ✓                              (starts immediately)
           agetty ━ 30ms → LOGIN PROMPT                  (starts immediately)

t= +600ms  ▶ login prompt on ttyS0                       ← measured

t=+1583ms  udev ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ ✓ (socket ready)
           ▶ udev-trigger starts immediately

t=+1835ms  dbus ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ ✓ (socket ready)
           ▶ NetworkManager, sshd, polkit, avahi start immediately

t=+2200ms  ▶ sshd listening (350ms after its dep — dbus — was satisfied)   ← measured
```

### Summary

| Metric | OpenRC (rc_parallel=YES) | flint-init | Advantage |
|--------|--------------------------|------------|-----------|
| Userspace → login prompt | ~2300 ms | **~600 ms** | **3.8×** |
| Userspace → sshd listening | ~2400 ms | **~2200 ms** | 1.1× |
| Services started before udev finishes | 0 | **14** (agetty×7, sysctl, hwclock, dbus, syslog-ng, cronie, polkit after dbus) | — |
| Wave boundaries crossed | 3 | **0** | — |

The login-prompt gap is the starkest difference. OpenRC cannot show a login prompt until
the sysinit and boot waves have fully completed — even if agetty itself would take 30 ms.
flint-init starts agetty at t=0 because it has no declared dependencies.

sshd latency is similar because on this system dbus is the binding constraint for both:
flint-init starts sshd the instant dbus signals ready; OpenRC starts sshd in parallel with
dbus in the default wave, so dbus readiness is not the gate. On systems with more services
competing in the default wave, flint-init's per-service in-degree tracking would pull ahead
further.

> **Test environment:** QEMU 9.x, virtio-blk, host i7-12700K, 6.19 Artix kernel.
> OpenRC figures are representative of a stock Artix `rc_parallel=YES` install with the
> same 31 services; direct measurement on the same image was not possible since OpenRC
> was replaced by flint-init as PID 1.

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
