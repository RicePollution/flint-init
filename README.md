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

## Real Boot — Stage 4 (v0.4.0)

flint-init booted a real Artix Linux QEMU disk image as PID 1, replacing OpenRC,
with five real daemons managed from `/etc/flint/services/`:

```
[pid1] /dev already mounted by kernel (DEVTMPFS_MOUNT=y), skipping
[pid1] mounted /proc, /sys, /dev
[flint] loaded 5 service(s)
[ctl] listening on /run/flint/ctl.sock
[flint] starting: syslog-ng
[flint] starting: dbus
[flint] starting: agetty
[ready] dbus socket accepting connections: "/run/dbus/system_bus_socket"
[ready] dbus ready (socket): "/run/dbus/system_bus_socket"
[flint] starting: sshd
[flint] starting: networkmanager
Server listening on :: port 22.
Server listening on 0.0.0.0 port 22.

Artix Linux 6.19.11-artix1-1 (ttyS0)

artixlinux login: root (automatic login)
```

syslog-ng, dbus, agetty, sshd, and NetworkManager all start in parallel where their
deps allow. sshd and NetworkManager both unblock the moment dbus is ready — they don't
wait for each other.

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
