use serde::{Deserialize, Serialize};

/// Top-level service file — maps directly to a `.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDef {
    pub service: ServiceSection,
    pub deps: Option<DepsSection>,
    pub ready: Option<ReadySection>,
    pub resources: Option<ResourcesSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSection {
    pub name: String,
    pub exec: String,
    pub restart: Option<RestartPolicy>,
    /// Unix user to run the service as. Privileges are dropped before exec.
    pub user: Option<String>,
    /// Explicit argument list. When set, `exec` is the binary path and these
    /// are passed as argv[1..]. Avoids whitespace-splitting ambiguity.
    pub args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepsSection {
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadySection {
    pub strategy: ReadyStrategy,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReadyStrategy {
    Pidfile,
    Socket,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourcesSection {
    pub oom_score_adj: Option<i32>,
}

impl ServiceDef {
    /// Validate security-sensitive fields.
    ///
    /// `ready.path` must be absolute, must not contain `..`, and must be
    /// under `/run/` or `/var/run/` to prevent arbitrary file deletion.
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(ready) = &self.ready {
            if let Some(path) = &ready.path {
                let p = std::path::Path::new(path);
                if !p.is_absolute() {
                    anyhow::bail!(
                        "service '{}': ready.path '{}' must be absolute",
                        self.service.name, path
                    );
                }
                if p.components().any(|c| c == std::path::Component::ParentDir) {
                    anyhow::bail!(
                        "service '{}': ready.path '{}' contains '..' traversal",
                        self.service.name, path
                    );
                }
                if !path.starts_with("/run/") && !path.starts_with("/var/run/") {
                    anyhow::bail!(
                        "service '{}': ready.path '{}' must be under /run/ or /var/run/",
                        self.service.name, path
                    );
                }
            }
        }
        Ok(())
    }
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
            def.validate()
                .map_err(|e| anyhow::anyhow!("Failed to validate {:?}: {}", path, e))?;
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
    fn service_def_bincode_round_trips() {
        let original: ServiceDef = toml::from_str(FULL_TOML).unwrap();
        let bytes = bincode::serialize(&original).expect("serialize failed");
        let restored: ServiceDef = bincode::deserialize(&bytes).expect("deserialize failed");
        assert_eq!(restored.service.name, original.service.name);
        assert_eq!(restored.service.exec, original.service.exec);
        assert_eq!(restored.service.restart, original.service.restart);
        assert_eq!(
            restored.deps.as_ref().unwrap().needs,
            original.deps.as_ref().unwrap().needs
        );
    }

    #[test]
    fn service_section_parses_user_field() {
        let toml = "[service]\nname = \"sshd\"\nexec = \"/usr/sbin/sshd\"\nuser = \"nobody\"\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        assert_eq!(def.service.user.as_deref(), Some("nobody"));
    }

    #[test]
    fn service_section_parses_args_array() {
        let toml = "[service]\nname = \"dbus\"\nexec = \"/usr/bin/dbus-daemon\"\nargs = [\"--system\", \"--nofork\"]\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        assert_eq!(
            def.service.args.as_deref(),
            Some(&["--system".to_string(), "--nofork".to_string()][..])
        );
    }

    #[test]
    fn validate_accepts_ready_path_under_run() {
        let toml = "[service]\nname = \"sshd\"\nexec = \"/usr/sbin/sshd\"\n[ready]\nstrategy = \"pidfile\"\npath = \"/run/sshd.pid\"\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        assert!(def.validate().is_ok());
    }

    #[test]
    fn validate_accepts_ready_path_under_var_run() {
        let toml = "[service]\nname = \"nm\"\nexec = \"/usr/sbin/NetworkManager\"\n[ready]\nstrategy = \"pidfile\"\npath = \"/var/run/NetworkManager.pid\"\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        assert!(def.validate().is_ok());
    }

    #[test]
    fn validate_rejects_ready_path_outside_run() {
        let toml = "[service]\nname = \"bad\"\nexec = \"/bin/bad\"\n[ready]\nstrategy = \"pidfile\"\npath = \"/etc/passwd\"\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        let err = def.validate().unwrap_err().to_string();
        assert!(err.contains("/etc/passwd"), "error should mention the path: {}", err);
    }

    #[test]
    fn validate_rejects_ready_path_with_traversal() {
        let toml = "[service]\nname = \"bad\"\nexec = \"/bin/bad\"\n[ready]\nstrategy = \"pidfile\"\npath = \"/run/../etc/shadow\"\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        assert!(def.validate().is_err());
    }

    #[test]
    fn validate_accepts_no_ready_section() {
        let toml = "[service]\nname = \"simple\"\nexec = \"/bin/true\"\n";
        let def: ServiceDef = toml::from_str(toml).expect("parse failed");
        assert!(def.validate().is_ok());
    }

    #[test]
    fn load_services_from_dir_rejects_invalid_ready_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "[service]\nname = \"bad\"\nexec = \"/bin/bad\"\n[ready]\nstrategy = \"pidfile\"\npath = \"/etc/hosts\"\n").unwrap();
        let result = load_services_from_dir(dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("/etc/hosts"), "error should mention path: {}", msg);
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
