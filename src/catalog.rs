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
