mod ctl_proto;
mod ctl_server;
mod executor;
mod fstab;
mod graph;
mod pid1;
mod ready;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use anyhow::{Context, Result};
use nix::sys::signal::{signal, SigHandler, Signal};

use flint_init::cache;
use flint_init::service::ReadyStrategy;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static REBOOT: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_shutdown(_: std::ffi::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_reboot(_: std::ffi::c_int) {
    REBOOT.store(true, Ordering::SeqCst);
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn main() -> Result<()> {
    pid1::setup().context("pid1 setup failed")?;

    // Install signal handlers before starting any services.
    unsafe {
        signal(Signal::SIGTERM, SigHandler::Handler(handle_shutdown))
            .context("failed to install SIGTERM handler")?;
        signal(Signal::SIGINT, SigHandler::Handler(handle_shutdown))
            .context("failed to install SIGINT handler")?;
        signal(Signal::SIGUSR1, SigHandler::Handler(handle_reboot))
            .context("failed to install SIGUSR1 handler")?;
    }

    let args: Vec<String> = std::env::args().collect();
    let default_dir = if std::process::id() == 1 {
        "/etc/flint/services"
    } else {
        "services/examples"
    };
    let services_dir = args.get(1).map(String::as_str).unwrap_or(default_dir);
    let dir = Path::new(services_dir);

    let manifest_path = dir.join(flint_init::cache::MANIFEST_FILENAME);
    let t0 = std::time::Instant::now();
    let services = cache::load_services_cached(dir, &manifest_path)
        .with_context(|| format!("loading services from {:?}", dir))?;
    let load_us = t0.elapsed().as_micros();

    if services.is_empty() {
        eprintln!("[flint] no services found in {:?}", dir);
        return Ok(());
    }

    eprintln!("[flint] loaded {} service(s) in {}µs", services.len(), load_us);

    let graph =
        graph::ServiceGraph::build(services.clone()).context("building dependency graph")?;

    let shared = ctl_proto::new_shared_state();
    ctl_server::start(shared.clone());

    let (ready_tx, ready_rx) = mpsc::channel::<String>();

    for svc in &services {
        if let Some(r) = &svc.ready {
            if let Some(path) = &r.path {
                // Delete any stale readiness file before spawning the watcher.
                // If the file exists from a previous run, the watcher's fast-path
                // would fire immediately and falsely signal the service as ready.
                let _ = std::fs::remove_file(path);

                let p = Path::new(path);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating ready dir {:?}", parent))?;
                }
                match r.strategy {
                    ReadyStrategy::Pidfile => {
                        ready::watch_pidfile(p, ready_tx.clone(), svc.service.name.clone());
                    }
                    ReadyStrategy::Socket => {
                        ready::watch_socket(p, ready_tx.clone(), svc.service.name.clone());
                    }
                }
            }
        }
    }
    drop(ready_tx);

    executor::run(graph, services, ready_rx, &SHUTDOWN, shared).context("executor failed")?;

    if std::process::id() == 1 {
        pid1::supervise_forever(REBOOT.load(Ordering::SeqCst));
    }

    Ok(())
}
