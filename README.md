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
the instant readiness is signalled, with no polling. For socket readiness, a connection
probe confirms the socket is actually accepting before unblocking dependents.

**Direct execution.** No shell fork per service. flint-init calls `execve` directly.

**Service restart.** Services with `restart = "on-failure"` or `restart = "always"` are
automatically respawned (up to 5 attempts). Permanently-failed services propagate failure
to their `needs`-dependents rather than blocking the boot indefinitely.

**Live status via flint-ctl.** A Unix socket at `/run/flint/ctl.sock` exposes service
state to the `flint-ctl` CLI. Query status or stop services while flint-init is running.

## Roadmap

| Stage | Description | Status |
|-------|-------------|--------|
| 1 | DAG executor, inotify readiness, parallel service spawning | Complete |
| 2 | PID 1 handoff, real service definitions, QEMU boot | Complete |
| 3 | Service restart logic, socket connection probing, flint-ctl | Complete |
| 4 | TBD | In progress |

## Stage 3 — Restart, Socket Probing, flint-ctl (v0.3.0)

### Service restart

Services are respawned according to their restart policy. `on-failure` restarts on
nonzero exit; `always` restarts unconditionally. After 5 failed attempts the service is
marked `Failed` and any `needs`-dependents are blocked.

QEMU test output showing a service failing twice then succeeding on attempt 3:

```
[flint] starting: restart-svc
[restart-test] attempt 1: FAILING (exit 1)
[flint] exited: restart-svc (code=1)
[flint] restarting: restart-svc (attempt 1/5)
[restart-test] attempt 2: FAILING (exit 1)
[flint] exited: restart-svc (code=1)
[flint] restarting: restart-svc (attempt 2/5)
[restart-test] attempt 3: SUCCESS (exit 0)
[flint] exited: restart-svc (code=0)
```

### Socket connection probing

`watch_socket` now calls `probe_unix_socket` after inotify fires: it loops
`UnixStream::connect` until success or a 5-second timeout. This prevents a service
from being declared ready the instant the socket file appears but before the daemon
is actually accepting. Fail-open: if the probe times out a warning is logged and
readiness is signalled anyway.

### flint-ctl

A control socket at `/run/flint/ctl.sock` accepts line-oriented JSON commands.
The `flint-ctl` binary wraps it with a simple CLI:

```
$ flint-ctl status
{
  "services": [
    { "name": "restart-svc", "state": "exited", "pid": null },
    { "name": "ctl-test",    "state": "running", "pid": 63 },
    { "name": "a",           "state": "exited",  "pid": null }
  ]
}

$ flint-ctl stop networkmanager
{ "ok": true }
```

---

## Stage 2 — QEMU Boot (v0.2.1)

flint-init boots a real Linux initramfs in QEMU as PID 1, starting and coordinating
four real Artix Linux daemons in dependency order:

```
udev → dbus → nm-priv-helper → NetworkManager
```

Console output from a successful boot:

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

**`/dev/stdout` missing from devtmpfs.**
`mkinitcpio` normally creates `/dev/stdout → /proc/self/fd/1` as part of its init
hooks. We don't run those hooks. Without it, any service configured to log to stdout
silently falls back to syslog. Added the standard `/dev/fd`, `/dev/stdin`,
`/dev/stdout`, `/dev/stderr` symlinks to `pid1::setup()`.

**NetworkManager doesn't write its pidfile in `--no-daemon` mode.**
When daemonizing normally, NM forks, the daemon writes the pidfile, the parent exits.
With `--no-daemon` in NM 1.56 the foreground process doesn't write it. Removed
`--no-daemon` from the service definition.

**nm-priv-helper can't be activated via D-Bus.**
NM 1.40+ requires `nm-priv-helper`. D-Bus would normally activate it via
`/usr/lib/dbus-daemon-launch-helper`, but that binary is setuid and unreadable — it
cannot be copied into an initramfs. Every activation entry also carries a
`SystemdService=` field which causes dbus-daemon to contact a non-existent systemd
and hang indefinitely.

Fix: `nm-priv-helper` is added as its own flint-init service. A bash wrapper launches
it in the background, waits for it to register its D-Bus name, writes a pidfile, then
waits for the process. By the time NetworkManager starts, nm-priv-helper is already
registered — no D-Bus activation needed.

**D-Bus activation entries with `SystemdService=`.**
The entire `/usr/share/dbus-1/system-services/` directory is removed from the
initramfs so activation attempts immediately return an error rather than hanging.

**dbus-daemon privilege drop.**
`<user>dbus</user>` is removed from `system.conf` so dbus-daemon stays root, which
also satisfies the D-Bus policy that only root can own `org.freedesktop.nm_priv_helper`.

---

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

## QEMU Tests

```bash
# Full real-service boot (udev, dbus, nm-priv-helper, NetworkManager):
bash scripts/qemu-test.sh

# Stage 3 integration test (restart logic + flint-ctl, no real daemons):
bash scripts/qemu-stage3-test.sh
```

Requirements: `cargo`, `qemu-system-x86_64`, `cpio`, `gzip`, `ldd`.
