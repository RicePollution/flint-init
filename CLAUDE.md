# flint-init — CLAUDE.md

## What This Is

flint-init is a parallel-first init system for Linux desktops. It replaces sequential
init systems (OpenRC, SysV) with a dependency-DAG-driven supervisor that starts services
the instant their dependencies are satisfied, not in conservative waves.

## Current Stage: Stage 1

Stage 1 proves the core idea: parallel wave execution with real readiness detection is
faster and more stable than sequential execution. We test against fake services — shell
scripts that sleep, then create a pidfile.

## Explicitly Out of Scope (Stage 1)

- PID 1 / kernel handoff
- Boot image / initramfs integration
- blitz-ctl (control CLI)
- /dev/shm caching of supervisor state
- Portability to non-Linux systems
- Socket-based readiness (pidfile only for now)
- Service restart logic (stub the field, don't implement)

## Build Order

1. `src/service.rs` — TOML types + serde deserialization
2. `src/graph.rs` — DAG, in-degree, ready queue, cycle detection
3. `src/executor.rs` — fork/exec, SIGCHLD, completion dispatch
4. `src/ready.rs` — inotify pidfile watching

## TOML Service Format

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

### Field Semantics

- `after`: ordering-only dependency (start after, but don't require success)
- `needs`: hard dependency (if dep fails, this service is blocked/failed)
- `ready.strategy`: `"pidfile"` | `"socket"` (socket not yet implemented)
- `ready.path`: path inotify watches for creation
- `restart`: `"always"` | `"on-failure"` | `"never"` (stored, not yet acted on)

## Key Design Decisions

### In-Degree Ready Queue (not wave batching)

OpenRC and similar systems batch services into waves and wait for a full wave to
complete before starting the next. flint-init uses per-service in-degree counters.
When any service becomes ready, its dependents' in-degrees are decremented immediately.
Any dependent reaching zero joins the ready queue at once. This eliminates wave-boundary
latency entirely.

### inotify Instead of Polling

Readiness is not "the script exited". It is "the service created its pidfile/socket".
We use Linux inotify to watch the parent directory and react the instant the file
appears, with zero polling overhead.

### Fake Services for Testing

`services/examples/` contains TOML definitions and companion shell scripts. Each script
sleeps for a configurable duration, then writes its own pidfile. This simulates real
service startup without requiring actual system daemons.

## Crate Choices

- `nix` — Linux syscalls (fork, exec, waitpid, signal)
- `serde` + `toml` — config deserialization
- `inotify` — inotify file watching
- `bincode` — serialization (reserved for future IPC/state caching)
- `thiserror` — typed error enums
- `anyhow` — top-level error propagation in main
