# flint-init

A parallel-first init system for Linux desktops.

## The Problem

Modern Linux desktop boots are slower than they need to be. Init systems like OpenRC
start services sequentially or in coarse waves, wait for scripts to exit rather than
checking if services are actually ready, and fork a shell for every service invocation.

The result: services that could start in parallel sit idle waiting for an entire wave
to finish, and a service that starts quickly doesn't unlock its dependents until the
slowest sibling in its wave catches up.

## What flint-init Does Differently

**True per-service parallelism.** Services are nodes in a dependency DAG. Any service
whose dependencies are satisfied starts immediately — no wave boundaries.

**Real readiness detection.** A service is "ready" when it creates its pidfile or
socket, not when its launcher script exits. flint-init uses Linux inotify to react
the instant readiness is signalled, with no polling.

**Direct execution.** No shell fork per service. flint-init calls `execve` directly.

## Roadmap

| Stage | Description | Status |
|-------|-------------|--------|
| 1 | DAG executor + inotify readiness, tested against fake services | Complete |
| 2 | PID 1 handoff, real service definitions, QEMU boot | Complete |
| 3 | blitz-ctl control socket, state persistence | Planned |
| 4 | Boot image integration | Planned |

## Stage 2 — QEMU Boot (v0.2.1)

flint-init now boots a real Linux initramfs in QEMU as PID 1, starting and
coordinating three real Artix Linux daemons in dependency order:

```
udev → dbus → nm-priv-helper → NetworkManager
```

Actual console output from a successful boot:

```
[pid1] mounted /proc, /sys, /dev
[flint] loaded 4 service(s)
[flint] starting: udev
[ready] udev ready (socket): "/run/udev/control"
[flint] ready (pidfile): udev
[flint] starting: dbus
[ready] dbus ready (socket): "/run/dbus/system_bus_socket"
[flint] ready (pidfile): dbus
[flint] starting: nm-priv-helper
[ready] nm-priv-helper ready (pidfile): "/run/nm-priv-helper.pid"
[flint] ready (pidfile): nm-priv-helper
[flint] starting: networkmanager
[ready] networkmanager ready (pidfile): "/run/NetworkManager/NetworkManager.pid"
[flint] ready (pidfile): networkmanager
[pid1] FLINT_ON_EXIT=halt — halting
```

Each service starts the instant its dependencies signal ready. No waves, no polling.

### What was fixed to make this work

Several non-obvious issues needed solving to boot real services in a minimal initramfs:

**`/dev/stdout` missing from devtmpfs.**
`mkinitcpio` normally creates `/dev/stdout → /proc/self/fd/1` as part of its init
hooks. We don't run those hooks. Without it, any service configured to log to stdout
silently falls back to syslog. Added the standard `/dev/fd`, `/dev/stdin`,
`/dev/stdout`, `/dev/stderr` symlinks to `pid1::setup()`.

**NetworkManager doesn't write its pidfile in `--no-daemon` mode.**
OpenRC doesn't use `--no-daemon` when starting NM. When daemonizing normally, NM forks,
the daemon writes the pidfile, the parent exits. With `--no-daemon` in NM 1.56 the
foreground process doesn't write it. Removed `--no-daemon` from the service definition.

**nm-priv-helper can't be activated via D-Bus.**
NM 1.40+ requires `nm-priv-helper` (privilege isolation helper). D-Bus would normally
activate it via `/usr/lib/dbus-daemon-launch-helper`, but that binary is
`---s--x---` (setuid, unreadable) — it cannot be copied into an initramfs. Every
activation entry in `/usr/share/dbus-1/system-services/` on Arch/Artix also carries a
`SystemdService=` field, which causes dbus-daemon to contact a non-existent systemd and
hang indefinitely.

Fix: `nm-priv-helper` is added as its own flint-init service that starts before
NetworkManager. A small bash wrapper launches it in the background, waits one second
for it to register its D-Bus name, writes a pidfile, then waits for the process. By
the time NetworkManager starts, nm-priv-helper is already registered — no D-Bus
activation needed at all.

**D-Bus activation entries with `SystemdService=`.**
The entire `/usr/share/dbus-1/system-services/` directory is removed from the
initramfs. Any activation attempt on a service not already running immediately returns
an error rather than hanging.

**dbus-daemon privilege drop.**
dbus-daemon normally drops to UID 81 (`dbus` user) via `<user>dbus</user>` in
`system.conf`. Removed so dbus-daemon stays root, which also satisfies the D-Bus
policy that only root can own `org.freedesktop.nm_priv_helper`.

## Service Definition Format

```toml
[service]
name = "networkmanager"
exec = "/usr/sbin/NetworkManager"
restart = "on-failure"

[deps]
after = ["dbus", "udev"]
needs = ["dbus"]

[ready]
strategy = "pidfile"
path = "/run/NetworkManager.pid"

[resources]
oom_score_adj = -100
```

## Building

```bash
cargo build
cargo test
```

## QEMU Test

```bash
bash scripts/qemu-test.sh
```

Builds a release binary, packages a minimal initramfs with udev, dbus,
nm-priv-helper, and NetworkManager, and boots it under QEMU. Pass `Ctrl-A X`
to exit at any time. The VM halts automatically once all services have signalled
ready (`FLINT_ON_EXIT=halt` is set on the kernel command line).

Requirements: `cargo`, `qemu-system-x86_64`, `cpio`, `gzip`, `ldd`.
