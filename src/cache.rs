use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::service::ServiceDef;

pub const MANIFEST_VERSION: u8 = 1;
pub const MANIFEST_FILENAME: &str = "services.bin";

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub filename: String,
    pub mtime_secs: u64,
    pub def: ServiceDef,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u8,
    pub entries: Vec<ManifestEntry>,
}

pub fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    let bytes = bincode::serialize(manifest)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_manifest(path: &Path) -> Result<Option<Manifest>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    match bincode::deserialize::<Manifest>(&bytes) {
        Ok(m) if m.version == MANIFEST_VERSION => Ok(Some(m)),
        _ => Ok(None),
    }
}

/// Returns all `.toml` filenames and their modification times from `dir`.
fn scan_toml_files(dir: &Path) -> Result<Vec<(String, SystemTime)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            let mtime = entry.metadata()?.modified()?;
            let filename = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("DirEntry has no filename: {:?}", path))?
                .to_string_lossy()
                .into_owned();
            files.push((filename, mtime));
        }
    }
    Ok(files)
}

/// Parses a single `.toml` file and returns the `ServiceDef`.
fn parse_toml(dir: &Path, filename: &str) -> Result<ServiceDef> {
    let path = dir.join(filename);
    let content = fs::read_to_string(&path)?;
    let content = content.trim_end_matches('\0');
    toml::from_str(content)
        .map_err(|e| anyhow::anyhow!("Failed to parse {:?}: {}", path, e))
}

/// Builds a fresh `Manifest` from all `.toml` files in `dir`.
fn build_manifest(dir: &Path, toml_files: &[(String, SystemTime)]) -> Result<Manifest> {
    let mut entries = Vec::new();
    for (filename, mtime) in toml_files {
        let mtime_secs = mtime.duration_since(UNIX_EPOCH)?.as_secs();
        let def = parse_toml(dir, filename)?;
        entries.push(ManifestEntry { filename: filename.clone(), mtime_secs, def });
    }
    Ok(Manifest { version: MANIFEST_VERSION, entries })
}

/// Load service definitions, using and updating the binary manifest cache.
///
/// On a warm system (flint-watch has been running), all mtime_secs values will
/// match and the function returns without any TOML parsing.
pub fn load_services_cached(dir: &Path, manifest_path: &Path) -> Result<Vec<ServiceDef>> {
    let toml_files = scan_toml_files(dir)?;
    let on_disk: HashSet<&str> = toml_files.iter().map(|(n, _)| n.as_str()).collect();

    if let Some(mut manifest) = read_manifest(manifest_path)? {
        let mut changed = false;

        // Drop entries whose TOML files have been deleted.
        let before = manifest.entries.len();
        manifest.entries.retain(|e| on_disk.contains(e.filename.as_str()));
        if manifest.entries.len() != before {
            changed = true;
        }

        // Upsert stale or new entries.
        for (filename, mtime) in &toml_files {
            let mtime_secs = mtime.duration_since(UNIX_EPOCH)?.as_secs();
            let cached_mtime = manifest
                .entries
                .iter()
                .find(|e| &e.filename == filename)
                .map(|e| e.mtime_secs);
            if cached_mtime == Some(mtime_secs) {
                continue; // Entry is fresh — skip TOML parsing.
            }
            let def = parse_toml(dir, filename)?;
            if let Some(entry) = manifest.entries.iter_mut().find(|e| &e.filename == filename) {
                entry.mtime_secs = mtime_secs;
                entry.def = def;
            } else {
                manifest.entries.push(ManifestEntry {
                    filename: filename.clone(),
                    mtime_secs,
                    def,
                });
            }
            changed = true;
        }

        if changed {
            write_manifest(manifest_path, &manifest)?;
        }
        return Ok(manifest.entries.into_iter().map(|e| e.def).collect());
    }

    // No manifest — full build.
    let manifest = build_manifest(dir, &toml_files)?;
    write_manifest(manifest_path, &manifest)?;
    Ok(manifest.entries.into_iter().map(|e| e.def).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str) -> ManifestEntry {
        let toml = format!("[service]\nname = \"{name}\"\nexec = \"/bin/{name}\"\n");
        let def: ServiceDef = toml::from_str(&toml).unwrap();
        ManifestEntry { filename: format!("{name}.toml"), mtime_secs: 1_000_000, def }
    }

    #[test]
    fn write_and_read_manifest_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("services.bin");
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            entries: vec![make_entry("foo"), make_entry("bar")],
        };
        write_manifest(&path, &manifest).unwrap();
        let restored = read_manifest(&path).unwrap().expect("expected Some");
        assert_eq!(restored.version, MANIFEST_VERSION);
        assert_eq!(restored.entries.len(), 2);
        assert_eq!(restored.entries[0].filename, "foo.toml");
        assert_eq!(restored.entries[1].def.service.name, "bar");
    }

    #[test]
    fn read_manifest_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("services.bin");
        let result = read_manifest(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_manifest_returns_none_for_wrong_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("services.bin");
        // Write a manifest with a wrong version byte
        let m = Manifest { version: 99, entries: vec![] };
        let bytes = bincode::serialize(&m).unwrap();
        std::fs::write(&path, bytes).unwrap();
        let result = read_manifest(&path).unwrap();
        assert!(result.is_none(), "wrong version should yield None");
    }

    #[test]
    fn write_manifest_is_atomic_tmp_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("services.bin");
        let tmp = path.with_extension("tmp");
        write_manifest(&path, &Manifest { version: MANIFEST_VERSION, entries: vec![] }).unwrap();
        assert!(path.exists(), "manifest should exist");
        assert!(!tmp.exists(), ".tmp should not persist");
    }

    fn write_toml(dir: &std::path::Path, name: &str, exec: &str) {
        let content = format!("[service]\nname = \"{name}\"\nexec = \"{exec}\"\n");
        std::fs::write(dir.join(format!("{name}.toml")), content).unwrap();
    }

    #[test]
    fn load_services_cached_builds_manifest_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(dir.path(), "alpha", "/bin/alpha");
        write_toml(dir.path(), "beta", "/bin/beta");
        let manifest_path = dir.path().join("services.bin");
        assert!(!manifest_path.exists());
        let services = load_services_cached(dir.path(), &manifest_path).unwrap();
        assert_eq!(services.len(), 2);
        assert!(manifest_path.exists(), "manifest should be written on first load");
    }

    #[test]
    fn load_services_cached_does_not_rewrite_fresh_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(dir.path(), "alpha", "/bin/alpha");
        let manifest_path = dir.path().join("services.bin");
        load_services_cached(dir.path(), &manifest_path).unwrap();
        let mtime_before = manifest_path.metadata().unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        load_services_cached(dir.path(), &manifest_path).unwrap();
        let mtime_after = manifest_path.metadata().unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "manifest should not be rewritten when fresh");
    }

    #[test]
    fn load_services_cached_adds_new_entry_on_second_call() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(dir.path(), "alpha", "/bin/alpha");
        let manifest_path = dir.path().join("services.bin");
        load_services_cached(dir.path(), &manifest_path).unwrap();
        write_toml(dir.path(), "beta", "/bin/beta");
        let services = load_services_cached(dir.path(), &manifest_path).unwrap();
        assert_eq!(services.len(), 2, "new entry should be picked up");
    }

    #[test]
    fn load_services_cached_removes_deleted_entry() {
        let dir = tempfile::tempdir().unwrap();
        write_toml(dir.path(), "alpha", "/bin/alpha");
        write_toml(dir.path(), "beta", "/bin/beta");
        let manifest_path = dir.path().join("services.bin");
        load_services_cached(dir.path(), &manifest_path).unwrap();
        std::fs::remove_file(dir.path().join("beta.toml")).unwrap();
        let services = load_services_cached(dir.path(), &manifest_path).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].service.name, "alpha");
    }
}
