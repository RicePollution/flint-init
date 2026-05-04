use serde::Deserialize;

use crate::service::ServiceDef;

#[derive(Debug, Deserialize, Default)]
pub struct FlintConfig {
    pub global: Option<GlobalSection>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlobalSection {
    /// The login user that desktop/session services run as.
    /// Substituted for `%U` in service `user` fields.
    pub session_user: Option<String>,
}

impl FlintConfig {
    pub fn session_user(&self) -> Option<&str> {
        self.global.as_ref()?.session_user.as_deref()
    }

    /// Expand `%U` in the `user` field of every service definition.
    ///
    /// Services that set `user = "%U"` will run as `session_user`.
    /// If no `session_user` is configured, `%U` services have their
    /// `user` field cleared (they will inherit the process owner).
    pub fn apply_to(&self, mut services: Vec<ServiceDef>) -> Vec<ServiceDef> {
        for svc in &mut services {
            if svc.service.user.as_deref() == Some("%U") {
                svc.service.user = self.session_user().map(|s| s.to_string());
            }
        }
        services
    }
}

pub fn load_config() -> FlintConfig {
    load_config_from("/etc/flint/config.toml")
}

pub fn load_config_from(path: &str) -> FlintConfig {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return FlintConfig::default(),
    };
    match toml::from_str(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("flint: warning: could not parse {}: {}", path, e);
            FlintConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ServiceDef, ServiceSection};

    fn svc_with_user(user: Option<&str>) -> ServiceDef {
        ServiceDef {
            service: ServiceSection {
                name: "test".to_string(),
                exec: "/bin/true".to_string(),
                restart: None,
                user: user.map(|s| s.to_string()),
                args: None,
            },
            deps: None,
            ready: None,
            resources: None,
        }
    }

    #[test]
    fn load_config_returns_default_when_file_missing() {
        let cfg = load_config_from("/nonexistent/flint/config.toml");
        assert!(cfg.session_user().is_none());
    }

    #[test]
    fn load_config_parses_session_user() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[global]\nsession_user = \"keegan\"\n").unwrap();
        let cfg = load_config_from(path.to_str().unwrap());
        assert_eq!(cfg.session_user(), Some("keegan"));
    }

    #[test]
    fn load_config_returns_default_on_bad_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not [ valid toml ][").unwrap();
        let cfg = load_config_from(path.to_str().unwrap());
        assert!(cfg.session_user().is_none());
    }

    #[test]
    fn apply_to_expands_percent_u_to_session_user() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[global]\nsession_user = \"keegan\"\n").unwrap();
        let cfg = load_config_from(path.to_str().unwrap());

        let services = vec![svc_with_user(Some("%U"))];
        let result = cfg.apply_to(services);
        assert_eq!(result[0].service.user.as_deref(), Some("keegan"));
    }

    #[test]
    fn apply_to_clears_percent_u_when_no_session_user_configured() {
        let cfg = FlintConfig::default();
        let services = vec![svc_with_user(Some("%U"))];
        let result = cfg.apply_to(services);
        // No session_user configured — %U services clear their user field
        // (process inherits the owner, typically whoever launched flint)
        assert_eq!(result[0].service.user, None);
    }

    #[test]
    fn apply_to_does_not_modify_explicit_user_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[global]\nsession_user = \"keegan\"\n").unwrap();
        let cfg = load_config_from(path.to_str().unwrap());

        let services = vec![
            svc_with_user(Some("postgres")),
            svc_with_user(Some("root")),
            svc_with_user(None),
        ];
        let result = cfg.apply_to(services);
        assert_eq!(result[0].service.user.as_deref(), Some("postgres"));
        assert_eq!(result[1].service.user.as_deref(), Some("root"));
        assert_eq!(result[2].service.user, None);
    }

    #[test]
    fn apply_to_expands_all_percent_u_services() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[global]\nsession_user = \"alice\"\n").unwrap();
        let cfg = load_config_from(path.to_str().unwrap());

        let services = vec![
            svc_with_user(Some("%U")),
            svc_with_user(Some("root")),
            svc_with_user(Some("%U")),
        ];
        let result = cfg.apply_to(services);
        assert_eq!(result[0].service.user.as_deref(), Some("alice"));
        assert_eq!(result[1].service.user.as_deref(), Some("root"));
        assert_eq!(result[2].service.user.as_deref(), Some("alice"));
    }
}
