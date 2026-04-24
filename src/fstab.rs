use std::path::{Path, PathBuf};

use anyhow::Result;
use nix::mount::MsFlags;

#[derive(Debug, Clone, PartialEq)]
pub struct FstabEntry {
    pub spec: String,
    pub file: String,
    pub vfstype: String,
    pub options: String,
}

const VIRTUAL_TYPES: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "cgroup",
    "cgroup2",
    "efivarfs",
    "bpf",
    "debugfs",
    "pstore",
    "securityfs",
    "hugetlbfs",
    "mqueue",
    "tracefs",
    "fusectl",
    "rootfs",
];

pub fn parse_fstab(content: &str) -> Vec<FstabEntry> {
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                return None;
            }
            Some(FstabEntry {
                spec: parts[0].to_string(),
                file: parts[1].to_string(),
                vfstype: parts[2].to_string(),
                options: parts[3].to_string(),
            })
        })
        .collect()
}

pub fn should_mount(entry: &FstabEntry) -> bool {
    if entry.file == "/" {
        return false;
    }
    if entry.options.split(',').any(|o| o.trim() == "noauto") {
        return false;
    }
    if VIRTUAL_TYPES.contains(&entry.vfstype.as_str()) {
        return false;
    }
    true
}

pub fn resolve_spec(spec: &str) -> PathBuf {
    if let Some(uuid) = spec.strip_prefix("UUID=") {
        Path::new("/dev/disk/by-uuid").join(uuid)
    } else if let Some(label) = spec.strip_prefix("LABEL=") {
        Path::new("/dev/disk/by-label").join(label)
    } else {
        PathBuf::from(spec)
    }
}

pub fn parse_flags(options: &str) -> (MsFlags, String) {
    let mut flags = MsFlags::empty();
    let mut data: Vec<&str> = Vec::new();
    for opt in options.split(',') {
        match opt.trim() {
            "ro" => flags |= MsFlags::MS_RDONLY,
            "rw" | "defaults" | "auto" | "noauto" => {}
            "noexec" => flags |= MsFlags::MS_NOEXEC,
            "nosuid" => flags |= MsFlags::MS_NOSUID,
            "nodev" => flags |= MsFlags::MS_NODEV,
            "relatime" => flags |= MsFlags::MS_RELATIME,
            "strictatime" => flags |= MsFlags::MS_STRICTATIME,
            "sync" => flags |= MsFlags::MS_SYNCHRONOUS,
            other if !other.is_empty() => data.push(other),
            _ => {}
        }
    }
    (flags, data.join(","))
}

pub fn mount_all() -> Result<()> {
    use nix::mount::mount;

    let content = match std::fs::read_to_string("/etc/fstab") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[fstab] warning: cannot read /etc/fstab: {}", e);
            return Ok(());
        }
    };

    for entry in parse_fstab(&content).into_iter().filter(should_mount) {
        let device = resolve_spec(&entry.spec);
        let (flags, data) = parse_flags(&entry.options);

        if let Err(e) = std::fs::create_dir_all(&entry.file) {
            eprintln!("[fstab] warning: cannot create {}: {}", entry.file, e);
        }

        let data_opt: Option<&str> = if data.is_empty() { None } else { Some(&data) };

        match mount(
            Some(device.as_path()),
            entry.file.as_str(),
            Some(entry.vfstype.as_str()),
            flags,
            data_opt,
        ) {
            Ok(()) => eprintln!("[fstab] mounted {} on {}", entry.spec, entry.file),
            Err(e) => eprintln!(
                "[fstab] warning: failed to mount {} on {}: {}",
                entry.spec, entry.file, e
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fstab_skips_comments_and_blanks() {
        let content = "
# This is a comment
UUID=abc123  /       ext4   defaults  0  1
UUID=def456  /boot   vfat   defaults  0  2

tmpfs        /tmp    tmpfs  defaults  0  0
";
        let entries = parse_fstab(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].spec, "UUID=abc123");
        assert_eq!(entries[0].file, "/");
        assert_eq!(entries[1].file, "/boot");
        assert_eq!(entries[2].vfstype, "tmpfs");
    }

    #[test]
    fn should_mount_skips_root() {
        let e = FstabEntry {
            spec: "UUID=abc".into(),
            file: "/".into(),
            vfstype: "ext4".into(),
            options: "defaults".into(),
        };
        assert!(!should_mount(&e));
    }

    #[test]
    fn should_mount_skips_virtual_types() {
        for vt in &["proc", "sysfs", "devtmpfs", "tmpfs", "cgroup2"] {
            let e = FstabEntry {
                spec: "none".into(),
                file: format!("/{}", vt),
                vfstype: vt.to_string(),
                options: "defaults".into(),
            };
            assert!(!should_mount(&e), "should skip vfstype={}", vt);
        }
    }

    #[test]
    fn should_mount_skips_noauto() {
        let e = FstabEntry {
            spec: "UUID=xyz".into(),
            file: "/mnt/backup".into(),
            vfstype: "ext4".into(),
            options: "noauto,defaults".into(),
        };
        assert!(!should_mount(&e));
    }

    #[test]
    fn should_mount_accepts_real_partitions() {
        let e = FstabEntry {
            spec: "UUID=abc".into(),
            file: "/home".into(),
            vfstype: "ext4".into(),
            options: "defaults".into(),
        };
        assert!(should_mount(&e));
    }

    #[test]
    fn resolve_spec_uuid() {
        let p = resolve_spec("UUID=1234-5678");
        assert_eq!(p, Path::new("/dev/disk/by-uuid/1234-5678"));
    }

    #[test]
    fn resolve_spec_label() {
        let p = resolve_spec("LABEL=boot");
        assert_eq!(p, Path::new("/dev/disk/by-label/boot"));
    }

    #[test]
    fn resolve_spec_device_path() {
        let p = resolve_spec("/dev/sda1");
        assert_eq!(p, Path::new("/dev/sda1"));
    }

    #[test]
    fn parse_flags_ro() {
        let (flags, data) = parse_flags("ro,noexec");
        assert!(flags.contains(MsFlags::MS_RDONLY));
        assert!(flags.contains(MsFlags::MS_NOEXEC));
        assert!(data.is_empty());
    }

    #[test]
    fn parse_flags_defaults_is_empty() {
        let (flags, data) = parse_flags("defaults");
        assert!(flags.is_empty());
        assert!(data.is_empty());
    }

    #[test]
    fn parse_flags_unknown_goes_to_data() {
        let (_, data) = parse_flags("defaults,uid=1000,gid=1000");
        assert_eq!(data, "uid=1000,gid=1000");
    }
}
