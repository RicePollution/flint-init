use std::collections::{HashMap, HashSet, VecDeque};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::catalog::CatalogEntry;
use crate::service::ServiceDef;

// ---------------------------------------------------------------------------
// Migration audit
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct MigrationAudit {
    covered: Vec<String>,
    missing: Vec<String>,
    unknown: Vec<String>,
}

/// Read enabled service names from the old init system.
/// Returns None when no known init system is detected.
fn detect_enabled_services() -> Option<Vec<String>> {
    // OpenRC: /etc/runlevels/default/ and /etc/runlevels/boot/
    let openrc_dirs = ["/etc/runlevels/default", "/etc/runlevels/boot"];
    if openrc_dirs.iter().any(|d| Path::new(d).exists()) {
        let mut services: HashSet<String> = HashSet::new();
        for dir in &openrc_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    services.insert(entry.file_name().to_string_lossy().to_string());
                }
            }
        }
        return Some(services.into_iter().collect());
    }

    // runit: /etc/runit/runsvdir/current/
    if Path::new("/etc/runit/runsvdir/current").exists() {
        if let Ok(entries) = std::fs::read_dir("/etc/runit/runsvdir/current") {
            let services: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            return Some(services);
        }
    }

    // s6: /etc/s6/adminsv/default/contents.d/
    if Path::new("/etc/s6/adminsv/default/contents.d").exists() {
        if let Ok(entries) = std::fs::read_dir("/etc/s6/adminsv/default/contents.d") {
            let services: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            return Some(services);
        }
    }

    None
}

fn installed_service_names(services_dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(services_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                names.insert(stem);
            }
        }
    }
    names
}

fn run_migration_audit(
    services_dir: &Path,
    catalog: &HashMap<String, CatalogEntry>,
) -> Option<MigrationAudit> {
    let enabled = detect_enabled_services()?;
    let installed = installed_service_names(services_dir);
    let mut audit = MigrationAudit::default();

    for svc in &enabled {
        if installed.contains(svc) {
            audit.covered.push(svc.clone());
        } else if catalog.contains_key(svc) {
            audit.missing.push(svc.clone());
        } else {
            audit.unknown.push(svc.clone());
        }
    }

    audit.covered.sort();
    audit.missing.sort();
    audit.unknown.sort();
    Some(audit)
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct PreflightResult {
    ok: Vec<String>,
    warnings: Vec<(String, String)>,
    errors: Vec<(String, String)>,
}

/// True if the path is ephemeral and should not be checked for writability.
fn is_ephemeral_path(path: &str) -> bool {
    path.starts_with("/run/") || path.starts_with("/var/run/") || path.starts_with("/dev/")
}

fn is_executable(path: &str) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Cycle detection via Kahn's algorithm. Returns true if a cycle exists.
fn has_cycle(services: &[ServiceDef]) -> bool {
    let names: HashSet<&str> = services.iter().map(|s| s.service.name.as_str()).collect();
    let mut in_degree: HashMap<&str, usize> = names.iter().map(|&n| (n, 0)).collect();
    // adjacency: dep -> list of services that depend on dep
    let mut adjacency: HashMap<&str, Vec<&str>> = names.iter().map(|&n| (n, vec![])).collect();

    for svc in services {
        if let Some(deps) = &svc.deps {
            let all_deps: HashSet<&str> = deps
                .after
                .iter()
                .chain(deps.needs.iter())
                .map(String::as_str)
                .collect();
            for dep in all_deps {
                if names.contains(dep) {
                    adjacency.entry(dep).or_default().push(svc.service.name.as_str());
                    *in_degree.entry(svc.service.name.as_str()).or_insert(0) += 1;
                }
            }
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&n, _)| n)
        .collect();
    let mut visited = 0usize;
    let mut tmp_degree = in_degree.clone();

    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(dependents) = adjacency.get(node) {
            for dep in dependents {
                let d = tmp_degree.entry(dep).or_insert(0);
                if *d > 0 {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    visited != services.len()
}

fn run_preflight(services_dir: &Path) -> PreflightResult {
    let mut result = PreflightResult::default();
    let entries = match std::fs::read_dir(services_dir) {
        Ok(e) => e,
        Err(_) => return result,
    };

    let mut services: Vec<ServiceDef> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                result.errors.push((name, format!("read error: {}", e)));
                continue;
            }
        };
        let content = content.trim_end_matches('\0');

        let def: ServiceDef = match toml::from_str(content) {
            Ok(d) => d,
            Err(e) => {
                result.errors.push((name, format!("parse error: {}", e)));
                continue;
            }
        };

        if let Err(e) = def.validate() {
            result.errors.push((name, e.to_string()));
            continue;
        }

        services.push(def);
    }

    let service_names: HashSet<&str> = services.iter().map(|s| s.service.name.as_str()).collect();

    // Cycle check across all parsed services
    let cycle = has_cycle(&services);

    for svc in &services {
        let name = svc.service.name.clone();
        let mut svc_errors: Vec<String> = Vec::new();
        let mut svc_warnings: Vec<String> = Vec::new();

        // Binary exists and is executable
        let exec_bin = svc.service.exec.split_whitespace().next().unwrap_or("");
        if !exec_bin.is_empty() && !is_executable(exec_bin) {
            svc_warnings.push(format!("{} not found", exec_bin));
        }

        // needs deps must all be installed
        if let Some(deps) = &svc.deps {
            for dep in &deps.needs {
                if !service_names.contains(dep.as_str()) {
                    svc_errors.push(format!("broken needs-dep \"{}\" (not installed)", dep));
                }
            }
            // after deps missing are a warning
            for dep in &deps.after {
                if !service_names.contains(dep.as_str()) {
                    svc_warnings.push(format!("unknown after-dep \"{}\"", dep));
                }
            }
        }

        // Writability of ready.path parent (skip ephemeral paths)
        if let Some(ready) = &svc.ready {
            if let Some(path) = &ready.path {
                if !is_ephemeral_path(path) {
                    if let Some(parent) = Path::new(path.as_str()).parent() {
                        if parent.exists() {
                            if let Ok(meta) = parent.metadata() {
                                if meta.permissions().mode() & 0o200 == 0 {
                                    svc_warnings.push(format!(
                                        "parent dir of {} may not be writable",
                                        path
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        if !svc_errors.is_empty() {
            result.errors.push((name, svc_errors.join("; ")));
        } else if !svc_warnings.is_empty() {
            result.warnings.push((name, svc_warnings.join("; ")));
        } else {
            result.ok.push(name);
        }
    }

    if cycle {
        result.errors.push(("(graph)".to_string(), "dependency cycle detected".to_string()));
    }

    result.ok.sort();
    result.warnings.sort_by(|a, b| a.0.cmp(&b.0));
    result.errors.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the full check command.
///
/// `catalog` is optional — pass `Some` to enable migration audit with
/// missing/unknown distinction. Pass `None` to skip the catalog-dependent
/// parts (e.g. when offline).
///
/// Returns `true` if there are no preflight errors (exit 0).
/// Returns `false` if there are errors (exit 1).
pub fn run_check(
    services_dir: &Path,
    catalog: Option<&HashMap<String, CatalogEntry>>,
) -> bool {
    if let Some(cat) = catalog {
        println!("[check] migration audit");
        match run_migration_audit(services_dir, cat) {
            None => {
                println!("  (no known init system detected — skipping migration audit)");
            }
            Some(audit) => {
                if !audit.covered.is_empty() {
                    println!(
                        "  covered   {}   ({} services)",
                        audit.covered.join(", "),
                        audit.covered.len()
                    );
                }
                if !audit.missing.is_empty() {
                    let cmd = format!("flint-ctl get {}", audit.missing.join(" "));
                    println!(
                        "  missing   {}   (in catalog — run: {})",
                        audit.missing.join(", "),
                        cmd
                    );
                }
                if !audit.unknown.is_empty() {
                    let cmds: Vec<String> = audit
                        .unknown
                        .iter()
                        .map(|s| format!("flint-ctl scaffold {}", s))
                        .collect();
                    println!(
                        "  unknown   {}   (not in catalog — run: {})",
                        audit.unknown.join(", "),
                        cmds.join("  |  ")
                    );
                }
                if audit.covered.is_empty() && audit.missing.is_empty() && audit.unknown.is_empty() {
                    println!("  (no enabled services detected from old init system)");
                }
            }
        }
        println!();
    }

    println!("[check] preflight");

    if !services_dir.exists() {
        println!(
            "  warning: {} does not exist — no services installed",
            services_dir.display()
        );
        return true;
    }

    let preflight = run_preflight(services_dir);

    if preflight.ok.is_empty() && preflight.warnings.is_empty() && preflight.errors.is_empty() {
        println!("  (no services found in {})", services_dir.display());
        return true;
    }

    if !preflight.ok.is_empty() {
        println!(
            "  ok        {}   ({} services)",
            preflight.ok.join(" "),
            preflight.ok.len()
        );
    }
    for (name, msg) in &preflight.warnings {
        println!("  warning   {}: {}", name, msg);
    }
    for (name, msg) in &preflight.errors {
        println!("  error     {}: {}", name, msg);
    }

    preflight.errors.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_toml(dir: &std::path::Path, name: &str, content: &str) {
        let path = dir.join(format!("{}.toml", name));
        let mut f = std::fs::File::create(path).unwrap();
        write!(f, "{}", content).unwrap();
    }

    #[test]
    fn preflight_ok_for_valid_service() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(
            dir.path(),
            "sshd",
            "[service]\nname = \"sshd\"\nexec = \"/bin/true\"\n",
        );
        let result = run_preflight(dir.path());
        assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    }

    #[test]
    fn preflight_warns_on_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(
            dir.path(),
            "myapp",
            "[service]\nname = \"myapp\"\nexec = \"/nonexistent/myapp\"\n",
        );
        let result = run_preflight(dir.path());
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty(), "expected binary-not-found warning");
    }

    #[test]
    fn preflight_errors_on_broken_needs_dep() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(
            dir.path(),
            "myapp",
            "[service]\nname = \"myapp\"\nexec = \"/bin/true\"\n[deps]\nneeds = [\"redis\"]\n",
        );
        let result = run_preflight(dir.path());
        assert!(!result.errors.is_empty(), "expected error for missing needs-dep");
        assert!(result.errors[0].1.contains("redis"));
    }

    #[test]
    fn preflight_errors_on_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(dir.path(), "bad", "this is not valid toml ][");
        let result = run_preflight(dir.path());
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn has_cycle_detects_two_node_cycle() {
        use crate::service::{DepsSection, ServiceSection};
        let make = |name: &str, after: &[&str]| ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: "/bin/true".to_string(),
                restart: None,
                user: None,
                args: None,
            },
            deps: Some(DepsSection {
                after: after.iter().map(|s| s.to_string()).collect(),
                needs: vec![],
            }),
            ready: None,
            resources: None,
        };
        let services = vec![make("a", &["b"]), make("b", &["a"])];
        assert!(has_cycle(&services));
    }

    #[test]
    fn has_cycle_no_cycle_for_chain() {
        use crate::service::{DepsSection, ServiceSection};
        let make = |name: &str, after: &[&str]| ServiceDef {
            service: ServiceSection {
                name: name.to_string(),
                exec: "/bin/true".to_string(),
                restart: None,
                user: None,
                args: None,
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
        };
        let services = vec![make("a", &[]), make("b", &["a"]), make("c", &["b"])];
        assert!(!has_cycle(&services));
    }

    #[test]
    fn is_ephemeral_path_matches_run_and_dev() {
        assert!(is_ephemeral_path("/run/foo.pid"));
        assert!(is_ephemeral_path("/var/run/foo.pid"));
        assert!(is_ephemeral_path("/dev/null"));
        assert!(!is_ephemeral_path("/etc/foo.pid"));
        assert!(!is_ephemeral_path("/tmp/foo.pid"));
    }
}
