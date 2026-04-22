use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::sync::mpsc::Receiver;

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
}

fn spawn_service(def: &ServiceDef) -> Result<Pid, ExecutorError> {
    let exec_path = CString::new(def.service.exec.as_str()).unwrap();

    match unsafe { fork() }? {
        ForkResult::Parent { child } => Ok(child),
        ForkResult::Child => {
            let _ = execv(&exec_path, &[exec_path.clone()]);
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
) -> Result<(), ExecutorError> {
    use std::sync::mpsc::TryRecvError;

    unsafe {
        let _ = signal(Signal::SIGCHLD, SigHandler::SigDfl);
    }

    let svc_by_name: HashMap<String, ServiceDef> =
        services.into_iter().map(|s| (s.service.name.clone(), s)).collect();

    let mut pid_to_name: HashMap<Pid, String> = HashMap::new();
    let mut ready_reported: HashSet<String> = HashSet::new();

    let mut to_start = graph.initially_ready();
    let total = graph.len();
    let mut completed = 0usize;

    loop {
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

    fn run_no_ready(graph: ServiceGraph, services: Vec<ServiceDef>) -> Result<(), ExecutorError> {
        let (_, rx) = mpsc::channel();
        run(graph, services, rx)
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
}
