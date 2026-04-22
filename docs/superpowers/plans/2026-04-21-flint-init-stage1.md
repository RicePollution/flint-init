# flint-init Stage 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scaffold flint-init — a parallel-first Linux init system — with a working dependency DAG, inotify-based readiness detection, and parallel service executor, proven against fake services.

**Architecture:** Services are defined in TOML and loaded into a DAG at startup. In-degree tracking drives the ready queue: nodes with in-degree zero start immediately, and each service completion decrements its dependents' in-degrees, firing new starts the instant their deps are satisfied. Readiness is detected via inotify on pidfiles/sockets, not process exit.

**Tech Stack:** Rust stable, `nix` (Linux syscalls), `serde` + `toml` (config), `inotify` (readiness watching), `bincode` (serialization, future use)

---

## File Map

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Workspace manifest, dependency versions |
| `CLAUDE.md` | Full project context for future sessions |
| `README.md` | Problem statement, approach, roadmap |
| `src/main.rs` | Entry point: load configs, build graph, run executor |
| `src/service.rs` | TOML-deserializable service definition types |
| `src/graph.rs` | Dependency DAG, in-degree computation, ready-queue logic |
| `src/executor.rs` | Fork/exec from ready queue, SIGCHLD handling, completion dispatch |
| `src/ready.rs` | inotify watcher for pidfile/socket readiness strategies |
| `services/examples/*.toml` | Fake services referencing shell scripts |
| `services/examples/scripts/*.sh` | Shell scripts that sleep then create a pidfile |

---

## Task 1: GitHub Repo + Project Scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/service.rs` (stub)
- Create: `src/graph.rs` (stub)
- Create: `src/executor.rs` (stub)
- Create: `src/ready.rs` (stub)
- Create: `CLAUDE.md`
- Create: `README.md`
- Create: `services/examples/.gitkeep`

- [ ] **Step 1: Create the GitHub repository**

```bash
gh repo create flint-init --public --description "Parallel-first init system for Linux" --clone=false
cd /home/keegan/Projects/flint-init
git init
git remote add origin git@github.com:$(gh api user --jq .login)/flint-init.git
```

- [ ] **Step 2: Write `Cargo.toml`**

```toml
[package]
name = "flint-init"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "flint-init"
path = "src/main.rs"

[dependencies]
serde = { version = "1", features = ["derive"] }
toml = "0.8"
nix = { version = "0.29", features = ["process", "signal", "fs"] }
inotify = "0.11"
bincode = "1"
thiserror = "1"
anyhow = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Write module stubs so the project compiles**

`src/main.rs`:
```rust
mod executor;
mod graph;
mod ready;
mod service;

fn main() {
    println!("flint-init stage 1");
}
```

`src/service.rs`:
```rust
// Service definition types — populated in Task 2
```

`src/graph.rs`:
```rust
// Dependency DAG — populated in Task 3
```

`src/executor.rs`:
```rust
// Parallel executor — populated in Task 4
```

`src/ready.rs`:
```rust
// Readiness detection — populated in Task 5
```

- [ ] **Step 4: Write `CLAUDE.md`**

```markdown
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
```

- [ ] **Step 5: Write `README.md`**

```markdown
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
```

- [ ] **Step 6: Create example service directories**

```bash
mkdir -p /home/keegan/Projects/flint-init/services/examples/scripts
touch /home/keegan/Projects/flint-init/services/examples/.gitkeep
```

- [ ] **Step 7: Verify the project compiles**

```bash
cd /home/keegan/Projects/flint-init
cargo build 2>&1
```
Expected: `Compiling flint-init v0.1.0` ... `Finished`

- [ ] **Step 8: Initial commit**

```bash
cd /home/keegan/Projects/flint-init
git add Cargo.toml Cargo.lock src/ CLAUDE.md README.md services/ docs/
git commit -m "chore: scaffold flint-init stage 1 project"
git branch -M main
git push -u origin main
```

---

## Task 2: Service Definition Types (`src/service.rs`)

**Files:**
- Modify: `src/service.rs`
- Create: `tests/service_tests.rs` (or inline `#[cfg(test)]`)

This task produces the Rust structs that a service TOML file deserializes into. No logic — just types.

- [ ] **Step 1: Write the failing test first**

Add to `src/service.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const FULL_TOML: &str = r#"
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
"#;

    #[test]
    fn deserializes_full_service_definition() {
        let def: ServiceDef = toml::from_str(FULL_TOML).expect("parse failed");
        assert_eq!(def.service.name, "networkmanager");
        assert_eq!(def.service.exec, "/usr/sbin/NetworkManager");
        assert_eq!(def.service.restart, Some(RestartPolicy::OnFailure));
        assert_eq!(def.deps.as_ref().unwrap().after, vec!["dbus", "udev"]);
        assert_eq!(def.deps.as_ref().unwrap().needs, vec!["dbus"]);
        assert_eq!(def.ready.as_ref().unwrap().strategy, ReadyStrategy::Pidfile);
        assert_eq!(def.ready.as_ref().unwrap().path, Some("/run/NetworkManager.pid".to_string()));
        assert_eq!(def.resources.as_ref().unwrap().oom_score_adj, Some(-100));
    }

    #[test]
    fn deserializes_minimal_service_definition() {
        let minimal = r#"
[service]
name = "foo"
exec = "/bin/foo"
"#;
        let def: ServiceDef = toml::from_str(minimal).expect("parse failed");
        assert_eq!(def.service.name, "foo");
        assert!(def.deps.is_none());
        assert!(def.ready.is_none());
        assert!(def.resources.is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/keegan/Projects/flint-init
cargo test 2>&1
```
Expected: compile error — `ServiceDef`, `RestartPolicy`, `ReadyStrategy` not defined.

- [ ] **Step 3: Implement the types**

Replace `src/service.rs` with:

```rust
use serde::Deserialize;

/// Top-level service file — maps directly to a `.toml` file.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceDef {
    pub service: ServiceSection,
    pub deps: Option<DepsSection>,
    pub ready: Option<ReadySection>,
    pub resources: Option<ResourcesSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceSection {
    pub name: String,
    pub exec: String,
    pub restart: Option<RestartPolicy>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DepsSection {
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadySection {
    pub strategy: ReadyStrategy,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReadyStrategy {
    Pidfile,
    Socket,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ResourcesSection {
    pub oom_score_adj: Option<i32>,
}

/// Load all `.toml` files from a directory into a Vec<ServiceDef>.
pub fn load_services_from_dir(dir: &std::path::Path) -> anyhow::Result<Vec<ServiceDef>> {
    let mut services = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            let content = std::fs::read_to_string(&path)?;
            let def: ServiceDef = toml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Failed to parse {:?}: {}", path, e))?;
            services.push(def);
        }
    }
    Ok(services)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_TOML: &str = r#"
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
"#;

    #[test]
    fn deserializes_full_service_definition() {
        let def: ServiceDef = toml::from_str(FULL_TOML).expect("parse failed");
        assert_eq!(def.service.name, "networkmanager");
        assert_eq!(def.service.exec, "/usr/sbin/NetworkManager");
        assert_eq!(def.service.restart, Some(RestartPolicy::OnFailure));
        assert_eq!(def.deps.as_ref().unwrap().after, vec!["dbus", "udev"]);
        assert_eq!(def.deps.as_ref().unwrap().needs, vec!["dbus"]);
        assert_eq!(def.ready.as_ref().unwrap().strategy, ReadyStrategy::Pidfile);
        assert_eq!(def.ready.as_ref().unwrap().path, Some("/run/NetworkManager.pid".to_string()));
        assert_eq!(def.resources.as_ref().unwrap().oom_score_adj, Some(-100));
    }

    #[test]
    fn deserializes_minimal_service_definition() {
        let minimal = r#"
[service]
name = "foo"
exec = "/bin/foo"
"#;
        let def: ServiceDef = toml::from_str(minimal).expect("parse failed");
        assert_eq!(def.service.name, "foo");
        assert!(def.deps.is_none());
        assert!(def.ready.is_none());
        assert!(def.resources.is_none());
    }

    #[test]
    fn load_services_from_dir_reads_toml_files() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "[service]\nname = \"foo\"\nexec = \"/bin/foo\"\n").unwrap();
        let services = load_services_from_dir(dir.path()).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].service.name, "foo");
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /home/keegan/Projects/flint-init
cargo test service 2>&1
```
Expected: `test service::tests::deserializes_full_service_definition ... ok` (3 tests pass)

- [ ] **Step 5: Commit**

```bash
cd /home/keegan/Projects/flint-init
git add src/service.rs
git commit -m "feat: add service definition types with serde deserialization"
```

---

## Task 3: Dependency Graph (`src/graph.rs`)

**Files:**
- Modify: `src/graph.rs`

The graph module owns the DAG, in-degree tracking, and the ready-queue logic. It is pure logic — no I/O, no process spawning.

**Interface this task must expose (used by Tasks 4 & 5):**

```rust
pub struct ServiceGraph { ... }

impl ServiceGraph {
    /// Build from a list of service definitions. Returns Err if a cycle exists.
    pub fn build(services: Vec<ServiceDef>) -> Result<Self, GraphError>

    /// Names of services with in-degree zero — start these immediately.
    pub fn initially_ready(&self) -> Vec<String>

    /// Call when a service becomes ready. Returns newly unblocked service names.
    pub fn mark_ready(&mut self, name: &str) -> Vec<String>

    /// Total service count.
    pub fn len(&self) -> usize
}
```

- [ ] **Step 1: Write failing tests**

Add to `src/graph.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::*;

    fn make_service(name: &str, after: &[&str], needs: &[&str]) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: format!("/bin/{}", name),
                restart: None,
            },
            deps: if after.is_empty() && needs.is_empty() {
                None
            } else {
                Some(DepsSection {
                    after: after.iter().map(|s| s.to_string()).collect(),
                    needs: needs.iter().map(|s| s.to_string()).collect(),
                })
            },
            ready: None,
            resources: None,
        }
    }

    #[test]
    fn single_service_is_immediately_ready() {
        let services = vec![make_service("foo", &[], &[])];
        let graph = ServiceGraph::build(services).unwrap();
        assert_eq!(graph.initially_ready(), vec!["foo"]);
    }

    #[test]
    fn service_with_dep_not_initially_ready() {
        let services = vec![
            make_service("a", &[], &[]),
            make_service("b", &["a"], &[]),
        ];
        let graph = ServiceGraph::build(services).unwrap();
        let ready = graph.initially_ready();
        assert_eq!(ready, vec!["a"]);
    }

    #[test]
    fn marking_ready_unblocks_dependent() {
        let services = vec![
            make_service("a", &[], &[]),
            make_service("b", &["a"], &[]),
        ];
        let mut graph = ServiceGraph::build(services).unwrap();
        let unlocked = graph.mark_ready("a");
        assert_eq!(unlocked, vec!["b"]);
    }

    #[test]
    fn diamond_dependency_waits_for_both_parents() {
        // root -> left, root -> right, both -> leaf
        let services = vec![
            make_service("root", &[], &[]),
            make_service("left", &["root"], &[]),
            make_service("right", &["root"], &[]),
            make_service("leaf", &["left", "right"], &[]),
        ];
        let mut graph = ServiceGraph::build(services).unwrap();
        // only root starts
        let initially = graph.initially_ready();
        assert_eq!(initially, vec!["root"]);
        // completing root unblocks left and right
        let mut after_root = graph.mark_ready("root");
        after_root.sort();
        assert_eq!(after_root, vec!["left", "right"]);
        // completing left does NOT yet unblock leaf
        let after_left = graph.mark_ready("left");
        assert!(after_left.is_empty(), "leaf should still wait for right: {:?}", after_left);
        // completing right unblocks leaf
        let after_right = graph.mark_ready("right");
        assert_eq!(after_right, vec!["leaf"]);
    }

    #[test]
    fn cycle_is_detected() {
        let services = vec![
            make_service("a", &["b"], &[]),
            make_service("b", &["a"], &[]),
        ];
        let result = ServiceGraph::build(services);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle"), "expected cycle error, got: {}", err);
    }

    #[test]
    fn self_loop_is_detected() {
        let services = vec![make_service("a", &["a"], &[])];
        let result = ServiceGraph::build(services);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/keegan/Projects/flint-init
cargo test graph 2>&1
```
Expected: compile error — `ServiceGraph`, `GraphError` not defined.

- [ ] **Step 3: Implement `src/graph.rs`**

```rust
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

use crate::service::ServiceDef;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("dependency cycle detected involving service(s): {0}")]
    Cycle(String),
    #[error("unknown dependency '{dep}' referenced by '{service}'")]
    UnknownDep { service: String, dep: String },
}

/// Dependency DAG with in-degree tracking for the parallel ready queue.
pub struct ServiceGraph {
    /// All service definitions, keyed by name.
    services: HashMap<String, ServiceDef>,
    /// Edges: name -> set of services that depend on it (i.e., reverse adjacency).
    dependents: HashMap<String, Vec<String>>,
    /// Current in-degree for each service (decremented as deps become ready).
    in_degree: HashMap<String, usize>,
}

impl ServiceGraph {
    /// Build the graph from a list of service definitions.
    ///
    /// Returns `Err(GraphError::Cycle)` if the dependency graph contains a cycle.
    /// Returns `Err(GraphError::UnknownDep)` if any dep name is not in the service list.
    pub fn build(services: Vec<ServiceDef>) -> Result<Self, GraphError> {
        let service_names: HashSet<String> =
            services.iter().map(|s| s.service.name.clone()).collect();

        let mut service_map: HashMap<String, ServiceDef> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut in_degree: HashMap<String, usize> = HashMap::new();

        for svc in &services {
            service_map.insert(svc.service.name.clone(), svc.clone());
            in_degree.insert(svc.service.name.clone(), 0);
            dependents.entry(svc.service.name.clone()).or_default();
        }

        // Forward edges: for each (a after b), b -> a
        for svc in &services {
            let all_deps: Vec<String> = svc
                .deps
                .as_ref()
                .map(|d| {
                    d.after
                        .iter()
                        .chain(d.needs.iter())
                        .cloned()
                        .collect::<HashSet<_>>() // deduplicate after+needs overlap
                        .into_iter()
                        .collect()
                })
                .unwrap_or_default();

            for dep in &all_deps {
                if !service_names.contains(dep) {
                    return Err(GraphError::UnknownDep {
                        service: svc.service.name.clone(),
                        dep: dep.clone(),
                    });
                }
                dependents.entry(dep.clone()).or_default().push(svc.service.name.clone());
                *in_degree.entry(svc.service.name.clone()).or_insert(0) += 1;
            }
        }

        // Cycle detection via Kahn's algorithm
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(n, _)| n.clone())
            .collect();
        let mut visited = 0usize;
        let mut tmp_degree = in_degree.clone();

        while let Some(node) = queue.pop_front() {
            visited += 1;
            if let Some(deps) = dependents.get(&node) {
                for dep in deps {
                    let d = tmp_degree.entry(dep.clone()).or_insert(0);
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        if visited != services.len() {
            // Find cycle participants for the error message
            let cycle_nodes: Vec<String> = tmp_degree
                .iter()
                .filter(|(_, &d)| d > 0)
                .map(|(n, _)| n.clone())
                .collect();
            return Err(GraphError::Cycle(cycle_nodes.join(", ")));
        }

        Ok(ServiceGraph { services: service_map, dependents, in_degree })
    }

    /// Services with in-degree zero — these start immediately at boot.
    pub fn initially_ready(&self) -> Vec<String> {
        let mut ready: Vec<String> = self
            .in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(n, _)| n.clone())
            .collect();
        ready.sort(); // deterministic ordering
        ready
    }

    /// Mark a service as ready (its pidfile appeared or it exited successfully).
    /// Returns the names of services that are now unblocked (their in-degree hit zero).
    pub fn mark_ready(&mut self, name: &str) -> Vec<String> {
        let mut newly_ready = Vec::new();
        if let Some(deps) = self.dependents.get(name).cloned() {
            for dep in deps {
                let d = self.in_degree.entry(dep.clone()).or_insert(0);
                if *d > 0 {
                    *d -= 1;
                    if *d == 0 {
                        newly_ready.push(dep);
                    }
                }
            }
        }
        newly_ready.sort();
        newly_ready
    }

    /// Returns the ServiceDef for a named service.
    pub fn get(&self, name: &str) -> Option<&ServiceDef> {
        self.services.get(name)
    }

    /// Total number of services in the graph.
    pub fn len(&self) -> usize {
        self.services.len()
    }

    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::*;

    fn make_service(name: &str, after: &[&str], needs: &[&str]) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: format!("/bin/{}", name),
                restart: None,
            },
            deps: if after.is_empty() && needs.is_empty() {
                None
            } else {
                Some(DepsSection {
                    after: after.iter().map(|s| s.to_string()).collect(),
                    needs: needs.iter().map(|s| s.to_string()).collect(),
                })
            },
            ready: None,
            resources: None,
        }
    }

    #[test]
    fn single_service_is_immediately_ready() {
        let services = vec![make_service("foo", &[], &[])];
        let graph = ServiceGraph::build(services).unwrap();
        assert_eq!(graph.initially_ready(), vec!["foo"]);
    }

    #[test]
    fn service_with_dep_not_initially_ready() {
        let services = vec![
            make_service("a", &[], &[]),
            make_service("b", &["a"], &[]),
        ];
        let graph = ServiceGraph::build(services).unwrap();
        let ready = graph.initially_ready();
        assert_eq!(ready, vec!["a"]);
    }

    #[test]
    fn marking_ready_unblocks_dependent() {
        let services = vec![
            make_service("a", &[], &[]),
            make_service("b", &["a"], &[]),
        ];
        let mut graph = ServiceGraph::build(services).unwrap();
        let unlocked = graph.mark_ready("a");
        assert_eq!(unlocked, vec!["b"]);
    }

    #[test]
    fn diamond_dependency_waits_for_both_parents() {
        let services = vec![
            make_service("root", &[], &[]),
            make_service("left", &["root"], &[]),
            make_service("right", &["root"], &[]),
            make_service("leaf", &["left", "right"], &[]),
        ];
        let mut graph = ServiceGraph::build(services).unwrap();
        let initially = graph.initially_ready();
        assert_eq!(initially, vec!["root"]);
        let mut after_root = graph.mark_ready("root");
        after_root.sort();
        assert_eq!(after_root, vec!["left", "right"]);
        let after_left = graph.mark_ready("left");
        assert!(after_left.is_empty(), "leaf should still wait for right: {:?}", after_left);
        let after_right = graph.mark_ready("right");
        assert_eq!(after_right, vec!["leaf"]);
    }

    #[test]
    fn cycle_is_detected() {
        let services = vec![
            make_service("a", &["b"], &[]),
            make_service("b", &["a"], &[]),
        ];
        let result = ServiceGraph::build(services);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle"), "expected cycle error, got: {}", err);
    }

    #[test]
    fn self_loop_is_detected() {
        let services = vec![make_service("a", &["a"], &[])];
        let result = ServiceGraph::build(services);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /home/keegan/Projects/flint-init
cargo test graph 2>&1
```
Expected: all 6 graph tests pass, 0 failures.

- [ ] **Step 5: Commit**

```bash
cd /home/keegan/Projects/flint-init
git add src/graph.rs
git commit -m "feat: dependency DAG with in-degree ready queue and cycle detection"
```

---

## Task 4: Fake Services for Testing (`services/examples/`)

**Files:**
- Create: `services/examples/alpha.toml`
- Create: `services/examples/beta.toml`
- Create: `services/examples/gamma.toml`
- Create: `services/examples/scripts/alpha.sh`
- Create: `services/examples/scripts/beta.sh`
- Create: `services/examples/scripts/gamma.sh`

Topology: alpha has no deps (starts immediately). beta depends on alpha. gamma depends on alpha. This exercises the diamond-wait path.

The scripts write a pidfile to `/tmp/flint-test/<name>.pid` and then sleep, simulating a running daemon.

- [ ] **Step 1: Write the shell scripts**

`services/examples/scripts/alpha.sh`:
```bash
#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test
echo $$ > /tmp/flint-test/alpha.pid
echo "[alpha] started, pid=$$"
sleep 30
```

`services/examples/scripts/beta.sh`:
```bash
#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test
echo $$ > /tmp/flint-test/beta.pid
echo "[beta] started, pid=$$"
sleep 30
```

`services/examples/scripts/gamma.sh`:
```bash
#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test
echo $$ > /tmp/flint-test/gamma.pid
echo "[gamma] started, pid=$$"
sleep 30
```

- [ ] **Step 2: Make scripts executable**

```bash
chmod +x /home/keegan/Projects/flint-init/services/examples/scripts/*.sh
```

- [ ] **Step 3: Write the TOML definitions**

`services/examples/alpha.toml`:
```toml
[service]
name = "alpha"
exec = "/home/keegan/Projects/flint-init/services/examples/scripts/alpha.sh"

[ready]
strategy = "pidfile"
path = "/tmp/flint-test/alpha.pid"
```

`services/examples/beta.toml`:
```toml
[service]
name = "beta"
exec = "/home/keegan/Projects/flint-init/services/examples/scripts/beta.sh"

[deps]
after = ["alpha"]
needs = ["alpha"]

[ready]
strategy = "pidfile"
path = "/tmp/flint-test/beta.pid"
```

`services/examples/gamma.toml`:
```toml
[service]
name = "gamma"
exec = "/home/keegan/Projects/flint-init/services/examples/scripts/gamma.sh"

[deps]
after = ["alpha"]
needs = ["alpha"]

[ready]
strategy = "pidfile"
path = "/tmp/flint-test/gamma.pid"
```

- [ ] **Step 4: Verify the TOML files parse**

```bash
cd /home/keegan/Projects/flint-init
cargo test load_services 2>&1
```
Expected: existing `load_services_from_dir_reads_toml_files` passes. Then add a quick manual check:

```bash
cd /home/keegan/Projects/flint-init
cat > /tmp/check_parse.rs << 'EOF'
# This is verified by running: cargo test -- --nocapture
EOF
cargo test service::tests::load_services_from_dir -- --nocapture 2>&1
```

- [ ] **Step 5: Commit**

```bash
cd /home/keegan/Projects/flint-init
git add services/
git commit -m "feat: add fake services for stage 1 testing (alpha->beta, alpha->gamma)"
```

---

## Task 5: Parallel Executor (`src/executor.rs`)

**Files:**
- Modify: `src/executor.rs`

The executor:
1. Receives the initial ready queue from the graph.
2. For each ready service, forks and execs it directly (no shell).
3. Waits for SIGCHLD via `nix::sys::wait::waitpid` in a loop.
4. When a process exits, notifies the graph and sends any newly unblocked services to the ready queue.
5. Stops when all services have been started (not when they all exit — for long-running daemons).

Note: In Stage 1, readiness detection (Task 6) and process exit are treated as equivalent for driving the ready queue. The executor fires `mark_ready` on exit. Task 6 (ready.rs) will intercept pidfile creation and call `mark_ready` earlier — the executor should be designed to accept readiness signals from either source.

- [ ] **Step 1: Write the failing test**

Add to `src/executor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ServiceGraph;
    use crate::service::*;

    fn make_service(name: &str, after: &[&str], exec: &str) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: exec.to_string(),
                restart: None,
            },
            deps: if after.is_empty() {
                None
            } else {
                Some(DepsSection {
                    after: after.iter().map(|s| s.to_string()).collect(),
                    needs: vec![],
                })
            },
            ready: None,
            resources: None,
        }
    }

    #[test]
    fn executes_single_service_and_waits() {
        // Use `true` as a service — it exits immediately with 0.
        let services = vec![make_service("true_svc", &[], "/usr/bin/true")];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        let result = run(graph, services);
        assert!(result.is_ok(), "executor failed: {:?}", result);
    }

    #[test]
    fn executes_chain_in_order() {
        // a -> b: a is `true`, b is `true`. Both must execute.
        let services = vec![
            make_service("a", &[], "/usr/bin/true"),
            make_service("b", &["a"], "/usr/bin/true"),
        ];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        let result = run(graph, services);
        assert!(result.is_ok(), "chained executor failed: {:?}", result);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/keegan/Projects/flint-init
cargo test executor 2>&1
```
Expected: compile error — `run` not defined.

- [ ] **Step 3: Implement `src/executor.rs`**

```rust
use std::collections::HashMap;
use std::ffi::CString;

use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{execv, fork, ForkResult, Pid};
use thiserror::Error;

use crate::graph::ServiceGraph;
use crate::service::ServiceDef;

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("fork failed: {0}")]
    Fork(#[from] nix::Error),
    #[error("exec failed for service '{name}': {source}")]
    Exec { name: String, source: nix::Error },
    #[error("service '{0}' exited with non-zero status {1}")]
    ExitFailure(String, i32),
}

/// Fork and exec a single service. Returns the child PID.
fn spawn_service(def: &ServiceDef) -> Result<Pid, ExecutorError> {
    let exec_path = CString::new(def.service.exec.as_str()).unwrap();

    match unsafe { fork() }? {
        ForkResult::Parent { child } => Ok(child),
        ForkResult::Child => {
            // execv replaces this process image. If it returns, it failed.
            let _ = execv(&exec_path, &[exec_path.clone()]);
            // If exec fails, exit the child so the parent can detect it.
            std::process::exit(127);
        }
    }
}

/// Run all services in the graph to completion.
///
/// Services are started in dependency order using the in-degree ready queue.
/// When a service exits, its dependents are unblocked.
///
/// This function returns when all services have been started and have exited.
/// Long-running daemons will keep this blocked — for Stage 1 (fake services
/// that exit quickly), this is fine.
pub fn run(mut graph: ServiceGraph, services: Vec<ServiceDef>) -> Result<(), ExecutorError> {
    // Ignore SIGCHLD so waitpid works correctly without races.
    unsafe {
        let _ = signal(Signal::SIGCHLD, SigHandler::SigDfl);
    }

    let service_map: HashMap<String, ServiceDef> =
        services.into_iter().map(|s| (s.service.name.clone(), s)).collect();

    // pid -> service name, for reaping
    let mut pid_to_name: HashMap<Pid, String> = HashMap::new();

    // Start the initially ready services
    let mut to_start = graph.initially_ready();
    let total = graph.len();
    let mut started = 0usize;
    let mut completed = 0usize;

    while completed < total {
        // Launch all services currently in the ready queue
        while let Some(name) = to_start.pop() {
            if let Some(def) = service_map.get(&name) {
                eprintln!("[flint] starting: {}", name);
                let pid = spawn_service(def)?;
                pid_to_name.insert(pid, name);
                started += 1;
            }
        }

        if completed == started {
            // Nothing running and nothing to start — shouldn't happen with a valid graph
            break;
        }

        // Reap one child (blocking wait)
        match waitpid(None, Some(WaitPidFlag::empty())) {
            Ok(WaitStatus::Exited(pid, code)) => {
                if let Some(name) = pid_to_name.remove(&pid) {
                    eprintln!("[flint] exited: {} (code={})", name, code);
                    // Mark ready in graph regardless of exit code for Stage 1
                    let newly_ready = graph.mark_ready(&name);
                    to_start.extend(newly_ready);
                    completed += 1;
                }
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                if let Some(name) = pid_to_name.remove(&pid) {
                    eprintln!("[flint] killed: {} (signal={:?})", name, sig);
                    let newly_ready = graph.mark_ready(&name);
                    to_start.extend(newly_ready);
                    completed += 1;
                }
            }
            Ok(_) => {}
            Err(nix::Error::ECHILD) => break, // no more children
            Err(e) => return Err(ExecutorError::Fork(e)),
        }
    }

    eprintln!("[flint] all {} services completed", completed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ServiceGraph;
    use crate::service::*;

    fn make_service(name: &str, after: &[&str], exec: &str) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: exec.to_string(),
                restart: None,
            },
            deps: if after.is_empty() {
                None
            } else {
                Some(DepsSection {
                    after: after.iter().map(|s| s.to_string()).collect(),
                    needs: vec![],
                })
            },
            ready: None,
            resources: None,
        }
    }

    #[test]
    fn executes_single_service_and_waits() {
        let services = vec![make_service("true_svc", &[], "/usr/bin/true")];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        let result = run(graph, services);
        assert!(result.is_ok(), "executor failed: {:?}", result);
    }

    #[test]
    fn executes_chain_in_order() {
        let services = vec![
            make_service("a", &[], "/usr/bin/true"),
            make_service("b", &["a"], "/usr/bin/true"),
        ];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        let result = run(graph, services);
        assert!(result.is_ok(), "chained executor failed: {:?}", result);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /home/keegan/Projects/flint-init
cargo test executor 2>&1
```
Expected: 2 executor tests pass.

- [ ] **Step 5: Commit**

```bash
cd /home/keegan/Projects/flint-init
git add src/executor.rs
git commit -m "feat: parallel executor with SIGCHLD reaping and in-degree dispatch"
```

---

## Task 6: Readiness Detection (`src/ready.rs`)

**Files:**
- Modify: `src/ready.rs`

The ready watcher uses Linux inotify to watch for pidfile creation. It runs on its own thread, sends a message over a channel when a file appears, and the executor uses those messages to call `mark_ready` earlier than process exit.

**Interface:**

```rust
pub fn watch_pidfile(path: &Path, tx: Sender<String>, service_name: String) -> JoinHandle<()>
```

The caller spawns one watcher thread per service that has `ready.strategy = "pidfile"`. When the pidfile appears, the thread sends the service name on `tx` and exits.

- [ ] **Step 1: Write the failing test**

Add to `src/ready.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn detects_pidfile_creation() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("test.pid");

        let (tx, rx) = mpsc::channel();
        let path = pidfile.clone();
        let _handle = watch_pidfile(&path, tx, "test-service".to_string());

        // Give the watcher a moment to set up inotify
        std::thread::sleep(Duration::from_millis(50));

        // Create the pidfile
        std::fs::write(&pidfile, "12345").unwrap();

        // Should receive the service name within 1 second
        let received = rx.recv_timeout(Duration::from_secs(1)).expect("no signal received");
        assert_eq!(received, "test-service");
    }

    #[test]
    fn already_existing_pidfile_is_detected_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("existing.pid");

        // Create the pidfile before starting the watcher
        std::fs::write(&pidfile, "99999").unwrap();

        let (tx, rx) = mpsc::channel();
        let _handle = watch_pidfile(&pidfile, tx, "existing-service".to_string());

        let received = rx.recv_timeout(Duration::from_secs(1)).expect("no signal for existing file");
        assert_eq!(received, "existing-service");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/keegan/Projects/flint-init
cargo test ready 2>&1
```
Expected: compile error — `watch_pidfile` not defined.

- [ ] **Step 3: Implement `src/ready.rs`**

```rust
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use inotify::{EventMask, Inotify, WatchMask};

/// Spawn a thread that watches `path` for creation using inotify.
/// When the file appears (or already exists), sends `service_name` on `tx`.
pub fn watch_pidfile(path: &Path, tx: Sender<String>, service_name: String) -> JoinHandle<()> {
    let path = path.to_path_buf();
    thread::spawn(move || {
        if watch_once(&path, &tx, &service_name).is_err() {
            eprintln!("[ready] watcher for {} failed", service_name);
        }
    })
}

fn watch_once(path: &PathBuf, tx: &Sender<String>, service_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fast path: file already exists.
    if path.exists() {
        tx.send(service_name.to_string())?;
        return Ok(());
    }

    let parent = path.parent().ok_or("pidfile path has no parent")?;
    let filename = path.file_name().ok_or("pidfile path has no filename")?;

    let mut inotify = Inotify::init()?;
    inotify.watches().add(
        parent,
        WatchMask::CREATE | WatchMask::MOVED_TO | WatchMask::CLOSE_WRITE,
    )?;

    // Double-check after adding the watch to avoid TOCTOU race.
    if path.exists() {
        tx.send(service_name.to_string())?;
        return Ok(());
    }

    let mut buffer = [0u8; 4096];
    loop {
        let events = inotify.read_events_blocking(&mut buffer)?;
        for event in events {
            if let Some(name) = event.name {
                if name == filename
                    && (event.mask.contains(EventMask::CREATE)
                        || event.mask.contains(EventMask::MOVED_TO)
                        || event.mask.contains(EventMask::CLOSE_WRITE))
                {
                    tx.send(service_name.to_string())?;
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn detects_pidfile_creation() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("test.pid");

        let (tx, rx) = mpsc::channel();
        let path = pidfile.clone();
        let _handle = watch_pidfile(&path, tx, "test-service".to_string());

        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(&pidfile, "12345").unwrap();

        let received = rx.recv_timeout(Duration::from_secs(1)).expect("no signal received");
        assert_eq!(received, "test-service");
    }

    #[test]
    fn already_existing_pidfile_is_detected_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("existing.pid");
        std::fs::write(&pidfile, "99999").unwrap();

        let (tx, rx) = mpsc::channel();
        let _handle = watch_pidfile(&pidfile, tx, "existing-service".to_string());

        let received = rx.recv_timeout(Duration::from_secs(1)).expect("no signal for existing file");
        assert_eq!(received, "existing-service");
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /home/keegan/Projects/flint-init
cargo test ready 2>&1
```
Expected: 2 ready tests pass.

- [ ] **Step 5: Commit**

```bash
cd /home/keegan/Projects/flint-init
git add src/ready.rs
git commit -m "feat: inotify-based pidfile readiness detection"
```

---

## Task 7: Wire Everything in `main.rs`

**Files:**
- Modify: `src/main.rs`

Connect the pieces: load services from a directory arg, build the graph, run the executor. The executor in this stage doesn't yet integrate `ready.rs` inline — that integration happens here via a select loop over the `ready` channel and the `waitpid` result. For Stage 1 simplicity, `main.rs` wires them together with a unified event loop.

- [ ] **Step 1: Implement `src/main.rs`**

```rust
mod executor;
mod graph;
mod ready;
mod service;

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;

use anyhow::{Context, Result};

use graph::ServiceGraph;
use service::{load_services_from_dir, ReadyStrategy, ServiceDef};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let services_dir = args.get(1).map(String::as_str).unwrap_or("services/examples");
    let dir = Path::new(services_dir);

    let services = load_services_from_dir(dir)
        .with_context(|| format!("loading services from {:?}", dir))?;

    if services.is_empty() {
        eprintln!("[flint] no services found in {:?}", dir);
        return Ok(());
    }

    eprintln!("[flint] loaded {} service(s)", services.len());

    let graph =
        ServiceGraph::build(services.clone()).context("building dependency graph")?;

    // Spawn inotify watchers for all services with pidfile readiness
    let (ready_tx, ready_rx) = mpsc::channel::<String>();
    let service_map: HashMap<String, ServiceDef> =
        services.iter().map(|s| (s.service.name.clone(), s.clone())).collect();

    for svc in &services {
        if let Some(ready) = &svc.ready {
            if ready.strategy == ReadyStrategy::Pidfile {
                if let Some(path) = &ready.path {
                    ready::watch_pidfile(
                        Path::new(path),
                        ready_tx.clone(),
                        svc.service.name.clone(),
                    );
                }
            }
        }
    }
    drop(ready_tx); // so the channel closes when all watchers are done

    executor::run(graph, services, ready_rx, service_map)
        .context("executor failed")?;

    Ok(())
}
```

**Note:** `executor::run` signature needs updating to accept `ready_rx` and `service_map`. Update `src/executor.rs` `run` function signature:

```rust
pub fn run(
    mut graph: ServiceGraph,
    services: Vec<ServiceDef>,
    ready_rx: std::sync::mpsc::Receiver<String>,
    service_map: HashMap<String, ServiceDef>,
) -> Result<(), ExecutorError>
```

The executor should now call `graph.mark_ready` on *both* pidfile signals (from `ready_rx.try_recv()`) and process exit signals (from `waitpid`), whichever comes first.

Full updated `executor.rs` `run` function — replace the existing `run` with:

```rust
pub fn run(
    mut graph: ServiceGraph,
    services: Vec<ServiceDef>,
    ready_rx: std::sync::mpsc::Receiver<String>,
    _service_map: HashMap<String, ServiceDef>,
) -> Result<(), ExecutorError> {
    use std::sync::mpsc::TryRecvError;

    unsafe {
        let _ = signal(Signal::SIGCHLD, SigHandler::SigDfl);
    }

    let svc_by_name: HashMap<String, ServiceDef> =
        services.into_iter().map(|s| (s.service.name.clone(), s)).collect();

    let mut pid_to_name: HashMap<Pid, String> = HashMap::new();
    let mut ready_reported: std::collections::HashSet<String> = Default::default();

    let mut to_start = graph.initially_ready();
    let total = graph.len();
    let mut completed = 0usize;

    loop {
        // Drain inotify readiness signals before blocking on waitpid
        loop {
            match ready_rx.try_recv() {
                Ok(name) => {
                    if ready_reported.insert(name.clone()) {
                        eprintln!("[flint] ready (pidfile): {}", name);
                        let newly = graph.mark_ready(&name);
                        to_start.extend(newly);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        // Launch all services currently unblocked
        while let Some(name) = to_start.pop() {
            if let Some(def) = svc_by_name.get(&name) {
                eprintln!("[flint] starting: {}", name);
                let pid = spawn_service(def)?;
                pid_to_name.insert(pid, name);
            }
        }

        if completed >= total {
            break;
        }

        if pid_to_name.is_empty() {
            // No running processes and nothing to start — deadlock or all done
            break;
        }

        // Block waiting for one child to exit
        match waitpid(None, Some(WaitPidFlag::empty())) {
            Ok(WaitStatus::Exited(pid, code)) => {
                if let Some(name) = pid_to_name.remove(&pid) {
                    eprintln!("[flint] exited: {} (code={})", name, code);
                    if ready_reported.insert(name.clone()) {
                        let newly = graph.mark_ready(&name);
                        to_start.extend(newly);
                    }
                    completed += 1;
                }
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                if let Some(name) = pid_to_name.remove(&pid) {
                    eprintln!("[flint] killed: {} (signal={:?})", name, sig);
                    if ready_reported.insert(name.clone()) {
                        let newly = graph.mark_ready(&name);
                        to_start.extend(newly);
                    }
                    completed += 1;
                }
            }
            Ok(_) => {}
            Err(nix::Error::ECHILD) => break,
            Err(e) => return Err(ExecutorError::Fork(e)),
        }
    }

    eprintln!("[flint] done ({} services)", completed);
    Ok(())
}
```

Also update the `executor.rs` tests to match the new signature (pass empty channels):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ServiceGraph;
    use crate::service::*;
    use std::sync::mpsc;

    fn make_service(name: &str, after: &[&str], exec: &str) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: exec.to_string(),
                restart: None,
            },
            deps: if after.is_empty() {
                None
            } else {
                Some(DepsSection {
                    after: after.iter().map(|s| s.to_string()).collect(),
                    needs: vec![],
                })
            },
            ready: None,
            resources: None,
        }
    }

    fn empty_run(graph: ServiceGraph, services: Vec<ServiceDef>) -> Result<(), ExecutorError> {
        let (_, rx) = mpsc::channel();
        let map = services.iter().map(|s| (s.service.name.clone(), s.clone())).collect();
        run(graph, services, rx, map)
    }

    #[test]
    fn executes_single_service_and_waits() {
        let services = vec![make_service("true_svc", &[], "/usr/bin/true")];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        assert!(empty_run(graph, services).is_ok());
    }

    #[test]
    fn executes_chain_in_order() {
        let services = vec![
            make_service("a", &[], "/usr/bin/true"),
            make_service("b", &["a"], "/usr/bin/true"),
        ];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        assert!(empty_run(graph, services).is_ok());
    }
}
```

- [ ] **Step 2: Run all tests**

```bash
cd /home/keegan/Projects/flint-init
cargo test 2>&1
```
Expected: all tests pass (service: 3, graph: 6, executor: 2, ready: 2).

- [ ] **Step 3: Run a smoke test with fake services**

```bash
cd /home/keegan/Projects/flint-init
cargo build 2>&1
# Clean up any leftover pidfiles
rm -f /tmp/flint-test/*.pid
# Run with a 3s timeout (scripts sleep 30, we just want to see startup)
timeout 3 ./target/debug/flint-init services/examples || true
```
Expected output (order of beta/gamma may vary):
```
[flint] loaded 3 service(s)
[flint] starting: alpha
[flint] ready (pidfile): alpha
[flint] starting: beta
[flint] starting: gamma
```

- [ ] **Step 4: Commit**

```bash
cd /home/keegan/Projects/flint-init
git add src/main.rs src/executor.rs
git commit -m "feat: wire executor, graph, and inotify readiness into main"
git push
```

---

## Self-Review

### Spec Coverage

| Requirement | Task |
|-------------|------|
| Services defined in TOML | Task 2 (service.rs) |
| Dependency DAG built at startup | Task 3 (graph.rs) |
| In-degree ready queue | Task 3 (graph.rs) |
| Services fire instantly when deps satisfied | Task 5 + Task 7 (executor + main) |
| inotify for pidfile readiness | Task 6 (ready.rs) |
| Cycle detection | Task 3 |
| Fake services for testing | Task 4 |
| Unit tests for graph | Task 3 |
| CLAUDE.md with full context | Task 1 |
| README with problem/roadmap | Task 1 |
| No PID 1, blitz-ctl, /dev/shm | Excluded by design |
| GitHub repo created | Task 1 |

### Placeholder Scan

No TBD/TODO items in code steps. All code is complete.

### Type Consistency

- `ServiceDef` defined in Task 2, used in Tasks 3, 5, 7 — consistent
- `ServiceGraph::build(Vec<ServiceDef>)` defined in Task 3, called in Tasks 5, 7 — consistent
- `ServiceGraph::mark_ready(&str) -> Vec<String>` defined in Task 3, called in Tasks 5, 7 — consistent
- `executor::run` signature updated in Task 7 and tests updated in same task — consistent
- `watch_pidfile(&Path, Sender<String>, String) -> JoinHandle<()>` defined in Task 6, called in Task 7 — consistent
