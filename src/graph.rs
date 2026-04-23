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
#[derive(Debug)]
pub struct ServiceGraph {
    /// All service definitions, keyed by name.
    services: HashMap<String, ServiceDef>,
    /// Edges: name -> set of services that depend on it (i.e., reverse adjacency).
    dependents: HashMap<String, Vec<String>>,
    /// Current in-degree for each service (decremented as deps become ready).
    in_degree: HashMap<String, usize>,
    /// Reverse adjacency covering only `needs` edges (hard dependencies).
    needs_dependents: HashMap<String, Vec<String>>,
}

impl ServiceGraph {
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

        for svc in &services {
            let all_deps: Vec<String> = svc
                .deps
                .as_ref()
                .map(|d| {
                    d.after
                        .iter()
                        .chain(d.needs.iter())
                        .cloned()
                        .collect::<HashSet<_>>()
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

        let mut needs_dependents: HashMap<String, Vec<String>> = HashMap::new();
        for svc in &services {
            needs_dependents.entry(svc.service.name.clone()).or_default();
        }
        for svc in &services {
            if let Some(deps) = &svc.deps {
                let unique_needs: std::collections::HashSet<&String> = deps.needs.iter().collect();
                for dep in unique_needs {
                    needs_dependents.entry(dep.clone()).or_default().push(svc.service.name.clone());
                }
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
            let cycle_nodes: Vec<String> = tmp_degree
                .iter()
                .filter(|(_, &d)| d > 0)
                .map(|(n, _)| n.clone())
                .collect();
            return Err(GraphError::Cycle(cycle_nodes.join(", ")));
        }

        Ok(ServiceGraph { services: service_map, dependents, in_degree, needs_dependents })
    }

    /// Services with in-degree zero — start these immediately at boot.
    pub fn initially_ready(&self) -> Vec<String> {
        let mut ready: Vec<String> = self
            .in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(n, _)| n.clone())
            .collect();
        ready.sort();
        ready
    }

    /// Mark a service ready. Returns names of services newly unblocked (in-degree hit zero).
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

    pub fn len(&self) -> usize {
        self.services.len()
    }

    pub fn needs_dependents(&self, name: &str) -> &[String] {
        self.needs_dependents.get(name).map(Vec::as_slice).unwrap_or(&[])
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
        assert_eq!(graph.initially_ready(), vec!["a"]);
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
        assert_eq!(graph.initially_ready(), vec!["root"]);
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

    #[test]
    fn needs_dependents_populated_correctly() {
        // b has both after AND needs on a; c has after-only
        let services = vec![
            make_service("a", &[], &[]),
            make_service("b", &["a"], &["a"]),
            make_service("c", &["a"], &[]),
        ];
        let graph = ServiceGraph::build(services).unwrap();
        let mut deps = graph.needs_dependents("a").to_vec();
        deps.sort();
        assert_eq!(deps, vec!["b"]);
    }

    #[test]
    fn needs_dependents_empty_for_after_only() {
        let services = vec![
            make_service("a", &[], &[]),
            make_service("b", &["a"], &[]),
        ];
        let graph = ServiceGraph::build(services).unwrap();
        assert_eq!(graph.needs_dependents("a"), &[] as &[String]);
    }

    #[test]
    fn needs_dependents_empty_for_unknown_service() {
        let services = vec![make_service("a", &[], &[])];
        let graph = ServiceGraph::build(services).unwrap();
        assert_eq!(graph.needs_dependents("nonexistent"), &[] as &[String]);
    }
}
