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
            let content = content.trim_end_matches('\0');
            let def: ServiceDef = toml::from_str(content)
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

    #[test]
    fn load_services_from_dir_errors_on_bad_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is not valid toml ][").unwrap();
        let result = load_services_from_dir(dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("bad.toml"), "error should mention filename, got: {}", msg);
    }
}
