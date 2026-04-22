mod executor;
mod graph;
mod ready;
mod service;

use std::path::Path;
use std::sync::mpsc;

use anyhow::{Context, Result};

use service::{load_services_from_dir, ReadyStrategy};

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
        graph::ServiceGraph::build(services.clone()).context("building dependency graph")?;

    // Spawn inotify watchers for all services with pidfile readiness
    let (ready_tx, ready_rx) = mpsc::channel::<String>();

    for svc in &services {
        if let Some(r) = &svc.ready {
            if r.strategy == ReadyStrategy::Pidfile {
                if let Some(path) = &r.path {
                    let p = Path::new(path);
                    // Ensure the parent directory exists so inotify can watch it
                    if let Some(parent) = p.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating pidfile dir {:?}", parent))?;
                    }
                    ready::watch_pidfile(
                        p,
                        ready_tx.clone(),
                        svc.service.name.clone(),
                    );
                }
            }
        }
    }
    drop(ready_tx); // close sender so channel disconnects when all watchers finish

    executor::run(graph, services, ready_rx).context("executor failed")?;

    Ok(())
}
