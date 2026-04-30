# flint-init

A parallel-first init system for Linux. Replaces OpenRC/SysV on desktops and servers
with a dependency-DAG supervisor that starts each service the instant its dependencies
are satisfied — not in waves.

Licensed under the [GNU General Public License v2.0](LICENSE).

---

## The Problem with Wave-Based Init

OpenRC and similar systems divide services into runlevels or waves. Every service in a
wave must finish before the next wave begins. A fast service (dbus, 80ms) sits idle
waiting for the slowest service in its wave before any of its dependents can start.

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
- `socket`: ready when the socket file appears **and** a `UnixStream::connect` probe
  succeeds. Fail-open: if the probe times out, flint-init logs a warning and signals
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
so it is running before any other service sees a config change.

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

### 8. Service Catalog

`flint-ctl get` fetches pre-built service definitions from the catalog hosted in this
repository, with interactive dependency resolution.

```
# List services available for your distro:
$ flint-ctl get --list
acpid                    ACPI event daemon
avahi                    mDNS/DNS-SD daemon
bluez                    Bluetooth protocol stack
dbus                     D-Bus system message bus
docker                   Docker container engine
networkmanager           Network connection manager
nginx                    Nginx web server
postgresql               PostgreSQL database server
sshd                     OpenSSH daemon
...

# Fetch and install a service (prompts for missing dependencies):
$ flint-ctl get sshd
installed: /etc/flint/services/sshd.toml

$ flint-ctl get networkmanager
installed: /etc/flint/services/networkmanager.toml
networkmanager requires dbus — fetch it too? [y/N] y
installed: /etc/flint/services/dbus.toml
networkmanager requires udev — fetch it too? [y/N] y
installed: /etc/flint/services/udev.toml
```

The catalog index (`catalog.toml`) lives at the repo root and lists all available
services with optional per-distro filtering. Fetched definitions are cached in
`/var/cache/flint/catalog.toml` (TTL: 24 hours).

Service definitions live under `services/`:

| Directory | Contents |
|-----------|----------|
| `services/global/` | Definitions that work on any distro (same binary paths everywhere) |
| `services/arch/` | Arch-specific overrides |
| `services/artix/` | Artix-specific overrides |
| `services/void/` | Void-specific overrides |
| `services/gentoo/` | Gentoo-specific overrides |

`flint-ctl get` tries `services/<distro>/<name>.toml` first and falls back to
`services/global/<name>.toml` if no distro-specific file exists. To contribute a new
service, add it to `services/global/` if the binary paths are the same across distros,
or to the appropriate distro directory if they differ.

If a service isn't in the catalog, `flint-ctl scaffold` generates a starter TOML from
the binary found in `$PATH`:

```
$ flint-ctl scaffold myapp
[service]
name = "myapp"
exec = "/usr/bin/myapp"   # verify foreground/nodaemon flags
restart = "on-failure"

# [deps]
# needs = ["dbus"]

# [ready]
# strategy = "pidfile"
# path = "/run/myapp/myapp.pid"
```

The output goes to stdout so you can review it before writing:

```bash
flint-ctl scaffold myapp > /etc/flint/services/myapp.toml
```

The commented sections are optional — fill in `[deps]` if the service requires others
to be ready first, and `[ready]` if it creates a pidfile or socket you want flint to
watch. The most important thing to verify is the `exec` line: most daemons need a
`--foreground` or `--no-daemon` flag to stay in the foreground so flint can supervise
them.

---

## Boot Time Comparison

Measured on an Artix Linux QEMU disk image (i7-12700K, KVM, kernel
`6.19.11-artix1-1`, virtio disk, 512 MB RAM, 2 vCPUs, serial console), averaged over 5
runs via `scripts/measure-flint-installed.sh`. OpenRC and systemd reference times are
from published benchmarks on real desktop hardware with comparable service sets.

The **userspace time** column measures from PID 1 exec to login prompt.

| System | Userspace (PID 1 → login) |
|--------|---------------------------|
| OpenRC 0.63.1 (rc_parallel=YES, ~25 services) | **~19,000 ms** |
| systemd 257 (~40 services) | **~15,000 ms** |
| **flint-init** (Artix, 32 services) | **759 ms** |

**~25× faster than OpenRC and ~20× faster than systemd.** flint-init's 759 ms includes
all 32 services beginning to start; agetty carries no dependencies so the login prompt
appears immediately while everything else initialises in the background.

OpenRC and systemd both gate the login prompt behind service startup completion.
flint-init avoids this entirely — agetty starts at t=0.

### What OpenRC is doing for those ~19 seconds

With `rc_parallel="YES"`, independent services in the same runlevel start concurrently.
Even so, the runlevel is a gate — the login prompt cannot appear until the entire
default runlevel finishes:

```
[openrc-init exec — t=0 relative]
  boot runlevel (parallel):
    hwclock, sysctl, loopback, hostname  ━━━ done  │
    fsck, localmount, mtab               ━━━━ done  │  boot runlevel must
    seedrng, esysusers, etmpfiles        ━━━━ done  │  complete before
    bootmisc, keymaps, net.lo            ━━━━ done  │  default begins
                                                    ▼
  default runlevel (parallel):
    dbus + elogind                       ━━━━━━━━━━━━━━━━━ done
    NetworkManager ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ done
    agetty.ttyS0   ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ waits for runlevel ─ ─ ─ ─ LOGIN ◄── t≈19s
```

### What systemd is doing for those ~15 seconds

```
[systemd[1] exec — t=0 relative]
  [ OK ] Created slices, sockets, mount units     │  early setup in parallel
  [ OK ] System Initialization target             │  must reach before continuing
  [ OK ] Coldplug udev devices                    │
  [ OK ] D-Bus System Message Bus                 │  each target must be reached
  [ OK ] Basic System target                      │  before dependent units start
  [ OK ] Network Manager                          │
  [ OK ] OpenSSH Daemon                           │
  [ OK ] Permit User Sessions                     │
  [ OK ] Serial Getty on ttyS0                    │
                                                  ▼
[login: — t≈15s]
```

### What flint-init is doing for those 760 ms

```
[PID 1 exec — t=0]
  ├─ config load  ━ 17–28 ms (warm binary cache)  ← replaces 114 ms cold TOML parse
  ├─ udev         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (socket ready at ~1500ms)
  ├─ dbus         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (socket ready at ~1800ms)
  ├─ syslog-ng    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ├─ flint-watch  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ (watches /etc/flint/services)
  ├─ sysctl       ━━ done                        ← no deps, runs in parallel
  ├─ hwclock      ━ done                         ← no deps, runs in parallel
  ├─ agetty       ━ LOGIN ◄── t=760ms            ← no deps, shows prompt immediately
  └─ agetty×6     ━ (tty1–tty6, also at t=0)

[udev ready → udev-trigger starts]
[dbus ready → sshd, NetworkManager, polkit start in parallel]
```

The login prompt is not held hostage by modules, clocks, or filesystems. agetty has no
dependencies in the service graph so it starts at t=0 and the user sees a prompt in
under a second while everything else initialises in the background.

> **flint-init measurement:** QEMU launched with `date +%s%N`, serial output captured to a
> file, polled until the login prompt appeared on ttyS0. Script: `scripts/measure-flint-installed.sh`.
> **OpenRC/systemd reference:** published benchmarks on real desktop hardware with
> `rc_parallel="YES"` (OpenRC) and default parallel targets (systemd), comparable service
> counts. Real-hardware numbers vary by ~±30% depending on storage speed and service set.

---

## Real Boot

flint-init booted a real Artix Linux QEMU disk image as PID 1, replacing OpenRC,
managing 32 services from `/etc/flint/services/` (warm binary cache):

```
[pid1] mounted /proc, /sys, /dev
[flint] loaded 32 service(s) in 16766µs
[ctl] listening on /run/flint/ctl.sock
[flint] starting: udev
[flint] starting: syslog-ng
[flint] starting: sysctl
[flint] starting: hwclock
[flint] starting: flint-watch
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

Artix Linux 6.19.11-artix1-1 (ttyS0)

artixlinux login: root (automatic login)

[flint-watch] manifest ready (32 entries), watching "/etc/flint/services"
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

The login prompt appears before udev has even finished initialising. 14 services start
at t=0 (no unsatisfied dependencies): udev, syslog-ng, sysctl, hwclock, flint-watch,
dbus, and all seven agetty instances. sshd, NetworkManager, polkit, and others unblock
the instant dbus signals ready.

---

## Stage History

| Stage | Description | Status |
|-------|-------------|--------|
| 1 | DAG executor, inotify readiness, parallel service spawning, unit tests | Complete |
| 2 | PID 1 handoff, real service definitions, QEMU initramfs boot | Complete |
| 3 | Service restart logic, socket connection probing, flint-ctl | Complete |
| 4 | fstab mounts, graceful unmount, real Artix disk image boot | Complete |
| 5 | Binary config cache, flint-watch inotify daemon, 4.9× faster warm boot | Complete |
| 6 | `install.sh` single-command installer, GitHub release, bats test suite | Complete |
| 7 | Service catalog — `flint-ctl get` with interactive dep resolution | Complete |

---

## Building

```bash
cargo build --release
cargo test          # 58 unit + integration tests, no root required
```

Requirements: Rust 1.75+, Linux (uses inotify, fork/exec, nix syscalls).

## Installing

```bash
# On a running Artix/Arch system:
curl -fsSL https://raw.githubusercontent.com/RicePollution/flint-init/main/install.sh | sudo bash

# Or into a mounted root (e.g. a QEMU disk image):
sudo bash install.sh --root /mnt/artix
```

Then add `init=/usr/sbin/flint-init` to your kernel parameters and reboot.

## QEMU Tests

```bash
# Full Artix disk image — 32 services, flint-init as PID 1:
sudo bash scripts/create-artix-vm.sh        # one-time setup, ~5 min
sudo bash scripts/install-flint-into-vm.sh  # install/update binaries
bash scripts/boot-artix-vm.sh               # boots and tails serial log

# Boot time measurements (5-run mean):
bash scripts/measure-flint-installed.sh artix-openrc.qcow2
sudo bash scripts/measure-openrc-installed.sh artix-openrc.qcow2

# Initramfs boot — udev, dbus, NetworkManager (no disk image required):
bash scripts/qemu-test.sh
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
