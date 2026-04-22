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
| 1 | DAG executor + inotify readiness, tested against fake services | In progress |
| 2 | PID 1 handoff, real service definitions | Planned |
| 3 | blitz-ctl control socket, state persistence | Planned |
| 4 | Boot image integration | Planned |

## Benchmarks

*(Numbers pending — Stage 1 establishes correctness, Stage 2 will benchmark against
OpenRC on a real boot sequence.)*

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
