use std::collections::HashMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CatalogEntry {
    pub description: String,
    pub distros: Option<Vec<String>>,
}

pub type Catalog = HashMap<String, CatalogEntry>;

pub fn detect_distro() -> String {
    detect_distro_from("/etc/os-release")
}

pub fn detect_distro_from(path: &str) -> String {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("flint-ctl: warning: {} not found, defaulting to artix", path);
            return "artix".to_string();
        }
    };
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("ID=") {
            return val.trim_matches('"').to_string();
        }
    }
    eprintln!("flint-ctl: warning: ID= not found in {}, defaulting to artix", path);
    "artix".to_string()
}

const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/RicePollution/flint-init/main/catalog.toml";
const CACHE_PATH: &str = "/var/cache/flint/catalog.toml";
const CACHE_TTL_SECS: u64 = 86400;

pub fn fetch_catalog() -> anyhow::Result<Catalog> {
    use std::time::Duration;

    let cache = std::path::Path::new(CACHE_PATH);

    if let Ok(meta) = cache.metadata() {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().unwrap_or(Duration::MAX) < Duration::from_secs(CACHE_TTL_SECS) {
                let content = std::fs::read_to_string(cache)?;
                return Ok(toml::from_str(&content)?);
            }
        }
    }

    let content = ureq::get(CATALOG_URL)
        .call()
        .map_err(|e| anyhow::anyhow!("failed to fetch catalog: {}", e))?
        .into_string()?;

    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(cache, &content) {
        eprintln!("flint-ctl: warning: could not write catalog cache: {}", e);
    }

    Ok(toml::from_str(&content)?)
}

pub fn fetch_service_toml(distro: &str, name: &str) -> anyhow::Result<String> {
    let distro_url = format!(
        "https://raw.githubusercontent.com/RicePollution/flint-init/main/services/{}/{}.toml",
        distro, name
    );
    let resp = ureq::get(&distro_url).call();
    match resp {
        Ok(r) => return Ok(r.into_string()?),
        Err(ureq::Error::Status(404, _)) => {}
        Err(e) => return Err(anyhow::anyhow!("failed to fetch service \"{}\": {}", name, e)),
    }

    let global_url = format!(
        "https://raw.githubusercontent.com/RicePollution/flint-init/main/services/global/{}.toml",
        name
    );
    let content = ureq::get(&global_url)
        .call()
        .map_err(|e| anyhow::anyhow!("failed to fetch service \"{}\": {}", name, e))?
        .into_string()?;
    Ok(content)
}

pub fn missing_deps(toml_str: &str, services_dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    let def: crate::service::ServiceDef = toml::from_str(toml_str)?;
    let needs = def.deps.map(|d| d.needs).unwrap_or_default();
    let missing = needs
        .into_iter()
        .filter(|dep| !services_dir.join(format!("{}.toml", dep)).exists())
        .collect();
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_distro_reads_id_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("os-release");
        std::fs::write(&path, "NAME=Artix\nID=artix\nID_LIKE=arch\n").unwrap();
        let result = detect_distro_from(path.to_str().unwrap());
        assert_eq!(result, "artix");
    }

    #[test]
    fn detect_distro_strips_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("os-release");
        std::fs::write(&path, "ID=\"arch\"\n").unwrap();
        let result = detect_distro_from(path.to_str().unwrap());
        assert_eq!(result, "arch");
    }

    #[test]
    fn detect_distro_missing_file_defaults_artix() {
        let result = detect_distro_from("/nonexistent/os-release");
        assert_eq!(result, "artix");
    }

    #[test]
    fn detect_distro_no_id_field_defaults_artix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("os-release");
        std::fs::write(&path, "NAME=SomeDistro\nVERSION=1\n").unwrap();
        let result = detect_distro_from(path.to_str().unwrap());
        assert_eq!(result, "artix");
    }

    #[test]
    fn catalog_entry_deserializes_without_distros() {
        let toml = r#"
[sshd]
description = "OpenSSH daemon"
"#;
        let catalog: Catalog = toml::from_str(toml).unwrap();
        assert_eq!(catalog["sshd"].description, "OpenSSH daemon");
        assert!(catalog["sshd"].distros.is_none());
    }

    #[test]
    fn missing_deps_returns_deps_not_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        // dbus.toml exists, networkmanager does not
        std::fs::write(dir.path().join("dbus.toml"), "[service]\nname=\"dbus\"\nexec=\"/bin/dbus\"\n").unwrap();

        let toml_str = r#"
[service]
name = "networkmanager"
exec = "/usr/bin/NetworkManager"

[deps]
needs = ["dbus", "udev"]
"#;
        let missing = missing_deps(toml_str, dir.path()).unwrap();
        assert_eq!(missing, vec!["udev"]);
    }

    #[test]
    fn missing_deps_no_deps_section_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let toml_str = "[service]\nname = \"foo\"\nexec = \"/bin/foo\"\n";
        let missing = missing_deps(toml_str, dir.path()).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn missing_deps_all_present_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dbus.toml"), "[service]\nname=\"dbus\"\nexec=\"/bin/dbus\"\n").unwrap();

        let toml_str = r#"
[service]
name = "sshd"
exec = "/usr/bin/sshd"

[deps]
needs = ["dbus"]
"#;
        let missing = missing_deps(toml_str, dir.path()).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn catalog_entry_deserializes_with_distros() {
        let toml = r#"
[agetty]
description = "TTY login prompt"
distros = ["artix"]
"#;
        let catalog: Catalog = toml::from_str(toml).unwrap();
        assert_eq!(catalog["agetty"].distros.as_ref().unwrap(), &["artix"]);
    }
}
