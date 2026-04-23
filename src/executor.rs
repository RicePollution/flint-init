use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{execv, fork, ForkResult, Pid};
use thiserror::Error;

use crate::graph::ServiceGraph;
use crate::service::ServiceDef;

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("fork failed: {0}")]
    Fork(#[from] nix::Error),
    #[error("service '{0}' has empty exec string")]
    EmptyExec(String),
}

fn spawn_service(def: &ServiceDef) -> Result<Pid, ExecutorError> {
    let parts: Vec<CString> = def
        .service
        .exec
        .split_whitespace()
        .map(|s| CString::new(s).unwrap())
        .collect();

    if parts.is_empty() {
        return Err(ExecutorError::EmptyExec(def.service.name.clone()));
    }

    let exec_path = parts[0].clone();

    match unsafe { fork() }? {
        ForkResult::Parent { child } => Ok(child),
        ForkResult::Child => {
            if let Some(adj) = def.resources.as_ref().and_then(|r| r.oom_score_adj) {
                let _ = std::fs::write("/proc/self/oom_score_adj", format!("{}\n", adj));
            }
            let _ = execv(&exec_path, &parts);
            std::process::exit(127);
        }
    }
}

/// Run all services in dependency order.
///
/// `ready_rx`: receives service names from inotify watchers when a pidfile appears.
/// When a service is signalled ready (via pidfile OR process exit), its dependents
/// are unblocked. Each service is only marked ready once (whichever comes first).
pub fn run(
    mut graph: ServiceGraph,
    services: Vec<ServiceDef>,
    ready_rx: Receiver<String>,
    shutdown: &AtomicBool,
) -> Result<(), ExecutorError> {
    use std::sync::mpsc::TryRecvError;

    let svc_by_name: HashMap<String, ServiceDef> =
        services.into_iter().map(|s| (s.service.name.clone(), s)).collect();

    let mut pid_to_name: HashMap<Pid, String> = HashMap::new();
    let mut ready_reported: HashSet<String> = HashSet::new();
    let mut blocked_services: HashSet<String> = HashSet::new();

    let mut to_start = graph.initially_ready();
    let total = graph.len();
    let mut completed = 0usize;

    loop {
        // Graceful shutdown: SIGTERM all tracked children and exit.
        if shutdown.load(Ordering::SeqCst) {
            for pid in pid_to_name.keys() {
                let _ = nix::sys::signal::kill(*pid, nix::sys::signal::Signal::SIGTERM);
            }
            break;
        }

        // Drain inotify readiness signals
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

        // Launch all currently unblocked services
        while let Some(name) = to_start.pop() {
            if blocked_services.contains(&name) {
                eprintln!("[flint] blocked: {}", name);
                for dep in graph.needs_dependents(&name) {
                    blocked_services.insert(dep.clone());
                }
                let newly = graph.mark_ready(&name);
                to_start.extend(newly);
                completed += 1;
                continue;
            }
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
            break;
        }

        // Wait up to 50 ms for a pidfile-ready signal, then poll children.
        // Using recv_timeout lets us wake as soon as a daemon writes its pidfile.
        // NOTE: This call blocks up to 50ms, which is also the maximum shutdown reaction
        // latency — the shutdown flag is only checked at the top of the next iteration.
        if let Ok(name) = ready_rx.recv_timeout(Duration::from_millis(50)) {
            if ready_reported.insert(name.clone()) {
                eprintln!("[flint] ready (pidfile): {}", name);
                let newly = graph.mark_ready(&name);
                to_start.extend(newly);
            }
        }

        // Poll each tracked child individually (WNOHANG) to avoid stealing
        // children that belong to other concurrent callers (e.g. test threads).
        let pids: Vec<Pid> = pid_to_name.keys().copied().collect();
        for pid in pids {
            match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, code)) => {
                    if let Some(name) = pid_to_name.remove(&pid) {
                        eprintln!("[flint] exited: {} (code={})", name, code);
                        completed += 1;
                        if code != 0 && !ready_reported.contains(&name) {
                            for dep in graph.needs_dependents(&name) {
                                blocked_services.insert(dep.clone());
                            }
                        }
                        if ready_reported.insert(name.clone()) {
                            let newly = graph.mark_ready(&name);
                            to_start.extend(newly);
                        }
                    }
                }
                Ok(WaitStatus::Signaled(pid, sig, _)) => {
                    if let Some(name) = pid_to_name.remove(&pid) {
                        eprintln!("[flint] killed: {} (signal={:?})", name, sig);
                        completed += 1;
                        if !ready_reported.contains(&name) {
                            for dep in graph.needs_dependents(&name) {
                                blocked_services.insert(dep.clone());
                            }
                        }
                        if ready_reported.insert(name.clone()) {
                            let newly = graph.mark_ready(&name);
                            to_start.extend(newly);
                        }
                    }
                }
                Ok(WaitStatus::StillAlive) => {}
                Ok(_) => {}
                Err(nix::Error::ECHILD) => {
                    // Process was already reaped (shouldn't happen in normal use).
                    pid_to_name.remove(&pid);
                    completed += 1;
                }
                Err(nix::Error::EINTR) => {}
                Err(e) => return Err(ExecutorError::Fork(e)),
            }
        }
    }

    eprintln!("[flint] done ({} services)", completed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ServiceGraph;
    use crate::service::*;
    use std::sync::atomic::{AtomicBool, Ordering};
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

    fn make_service_with_needs(name: &str, after: &[&str], needs: &[&str], exec: &str) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: exec.to_string(),
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
    fn failed_needs_dep_does_not_hang() {
        // "fail" exits 1; "blocked" needs "fail" → should be skipped, not hang
        let services = vec![
            make_service_with_needs("fail", &[], &[], "/usr/bin/false"),
            make_service_with_needs("blocked", &[], &["fail"], "/usr/bin/true"),
        ];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        assert!(run_no_ready(graph, services).is_ok());
    }

    #[test]
    fn after_only_dep_runs_despite_needs_failure() {
        // "fail" exits 1; "after_svc" only has `after = ["fail"]` — must still run
        let services = vec![
            make_service_with_needs("fail", &[], &[], "/usr/bin/false"),
            make_service_with_needs("after_svc", &["fail"], &[], "/usr/bin/true"),
        ];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        assert!(run_no_ready(graph, services).is_ok());
    }

    fn run_no_ready(graph: ServiceGraph, services: Vec<ServiceDef>) -> Result<(), ExecutorError> {
        let (_, rx) = mpsc::channel();
        static NO_SHUTDOWN: AtomicBool = AtomicBool::new(false);
        run(graph, services, rx, &NO_SHUTDOWN)
    }

    #[test]
    fn shutdown_flag_terminates_long_running_service() {
        use std::time::Duration;

        static TEST_SHUTDOWN: AtomicBool = AtomicBool::new(false);
        TEST_SHUTDOWN.store(false, Ordering::SeqCst);

        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            TEST_SHUTDOWN.store(true, Ordering::SeqCst);
        });

        let svc = make_service_with_needs("sleeper", &[], &[], "/usr/bin/sleep 100");
        let graph = ServiceGraph::build(vec![svc.clone()]).unwrap();
        let (_, rx) = mpsc::channel();
        let result = run(graph, vec![svc], rx, &TEST_SHUTDOWN);
        assert!(result.is_ok());
    }

    #[test]
    fn empty_exec_returns_error() {
        // whitespace-only exec splits to nothing → EmptyExec error
        let svc = make_service_with_needs("empty", &[], &[], "   ");
        let graph = ServiceGraph::build(vec![svc.clone()]).unwrap();
        let result = run_no_ready(graph, vec![svc]);
        assert!(matches!(result, Err(ExecutorError::EmptyExec(_))));
    }

    #[test]
    fn exec_with_args_runs_successfully() {
        // /usr/bin/env passes its argument list to execvp; "env true" runs /usr/bin/true
        let svc = make_service_with_needs("env_true", &[], &[], "/usr/bin/env true");
        let graph = ServiceGraph::build(vec![svc.clone()]).unwrap();
        assert!(run_no_ready(graph, vec![svc]).is_ok());
    }

    #[test]
    fn executes_single_service_and_waits() {
        let services = vec![make_service("true_svc", &[], "/usr/bin/true")];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        assert!(run_no_ready(graph, services).is_ok());
    }

    #[test]
    fn executes_chain_in_order() {
        let services = vec![
            make_service("a", &[], "/usr/bin/true"),
            make_service("b", &["a"], "/usr/bin/true"),
        ];
        let graph = ServiceGraph::build(services.clone()).unwrap();
        assert!(run_no_ready(graph, services).is_ok());
    }

    #[test]
    fn service_with_oom_score_adj_runs() {
        // positive oom_score_adj (more killable) needs no special capability
        let mut svc = make_service_with_needs("oom_svc", &[], &[], "/usr/bin/true");
        svc.resources = Some(crate::service::ResourcesSection { oom_score_adj: Some(100) });
        let graph = ServiceGraph::build(vec![svc.clone()]).unwrap();
        assert!(run_no_ready(graph, vec![svc]).is_ok());
    }
}
