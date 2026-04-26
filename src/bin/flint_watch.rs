use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::fs;

use anyhow::Result;
use inotify::{EventMask, Inotify, WatchMask};

use flint_init::cache::{
    load_services_cached, read_manifest, write_manifest, Manifest, ManifestEntry,
    MANIFEST_VERSION,
};
use flint_init::service::ServiceDef;

const DEFAULT_SERVICES_DIR: &str = "/etc/flint/services";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(DEFAULT_SERVICES_DIR));
    let manifest_path = dir.join("services.bin");

    // Ensure manifest is up to date at startup (handles gap while watcher was not running).
    let services = load_services_cached(&dir, &manifest_path)?;
    eprintln!("[flint-watch] manifest ready ({} entries), watching {:?}", services.len(), dir);

    let mut inotify = Inotify::init()?;
    inotify.watches().add(
        &dir,
        WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::DELETE,
    )?;

    let mut buffer = [0u8; 4096];
    loop {
        let events = inotify.read_events_blocking(&mut buffer)?;
        for event in events {
            let name = match event.name {
                Some(n) => n.to_string_lossy().into_owned(),
                None => continue,
            };
            if !name.ends_with(".toml") {
                continue;
            }

            let mut manifest = read_manifest(&manifest_path)?.unwrap_or(Manifest {
                version: MANIFEST_VERSION,
                entries: Vec::new(),
            });

            if event.mask.contains(EventMask::DELETE) {
                manifest.entries.retain(|e| e.filename != name);
                eprintln!("[flint-watch] removed entry for {}", name);
            } else {
                // CLOSE_WRITE or MOVED_TO
                match parse_and_stat(&dir, &name) {
                    Ok((def, mtime_secs)) => {
                        if let Some(entry) =
                            manifest.entries.iter_mut().find(|e| e.filename == name)
                        {
                            entry.mtime_secs = mtime_secs;
                            entry.def = def;
                        } else {
                            manifest.entries.push(ManifestEntry {
                                filename: name.clone(),
                                mtime_secs,
                                def,
                            });
                        }
                        eprintln!("[flint-watch] updated entry for {}", name);
                    }
                    Err(e) => {
                        // Leave old entry intact so boot still has a valid definition.
                        eprintln!("[flint-watch] warning: failed to parse {}: {}", name, e);
                    }
                }
            }

            write_manifest(&manifest_path, &manifest)?;
        }
    }
}

/// Parse a TOML file and return `(ServiceDef, mtime_secs)`.
fn parse_and_stat(dir: &Path, filename: &str) -> Result<(ServiceDef, u64)> {
    let path = dir.join(filename);
    let content = fs::read_to_string(&path)?;
    let content = content.trim_end_matches('\0');
    let def: ServiceDef = toml::from_str(content)
        .map_err(|e| anyhow::anyhow!("Failed to parse {:?}: {}", path, e))?;
    let mtime_secs = path
        .metadata()?
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_secs();
    Ok((def, mtime_secs))
}
