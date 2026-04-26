use std::fs;
use std::path::Path;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::service::ServiceDef;

pub const MANIFEST_VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
pub struct ManifestEntry {
    pub filename: String,
    pub mtime_secs: u64,
    pub def: ServiceDef,
}

#[derive(Serialize, Deserialize)]
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

pub fn load_services_cached(_dir: &Path, _manifest_path: &Path) -> Result<Vec<ServiceDef>> {
    unimplemented!()
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
}
