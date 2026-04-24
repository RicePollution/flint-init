use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

/// Mount essential kernel filesystems. No-op when not running as PID 1.
pub fn setup() -> Result<()> {
    if std::process::id() != 1 {
        return Ok(());
    }

    use nix::mount::{mount, MsFlags};

    // /proc — process information
    std::fs::create_dir_all("/proc").context("create /proc")?;
    mount(
        Some("proc"),
        "/proc",
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("mount /proc")?;

    // /sys — sysfs
    std::fs::create_dir_all("/sys").context("create /sys")?;
    mount(
        Some("sysfs"),
        "/sys",
        Some("sysfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("mount /sys")?;

    // /dev — tmpfs (udev will populate it).
    // When CONFIG_DEVTMPFS_MOUNT=y the kernel mounts devtmpfs before exec'ing
    // init, so we get EBUSY here — that is fine, skip the mount.
    std::fs::create_dir_all("/dev").context("create /dev")?;
    match mount(
        Some("devtmpfs"),
        "/dev",
        Some("devtmpfs"),
        MsFlags::empty(),
        None::<&str>,
    ) {
        Ok(()) => {}
        Err(nix::Error::EBUSY) => {
            eprintln!("[pid1] /dev already mounted by kernel (DEVTMPFS_MOUNT=y), skipping");
        }
        Err(e) => return Err(e).context("mount /dev")?,
    }

    // Standard /dev symlinks — mkinitcpio creates these in its init hooks;
    // we must do it ourselves since we bypass that layer.
    // Without /dev/stdout the NM logging backend silently falls back to syslog.
    let _ = std::os::unix::fs::symlink("/proc/self/fd", "/dev/fd");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/0", "/dev/stdin");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/1", "/dev/stdout");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/2", "/dev/stderr");

    // Mount remaining filesystems from /etc/fstab (skip root and virtual fs).
    if let Err(e) = crate::fstab::mount_all() {
        eprintln!("[pid1] fstab mount warning: {}", e);
    }

    eprintln!("[pid1] mounted /proc, /sys, /dev");
    Ok(())
}

fn unmount_all() {
    use nix::mount::{umount2, MntFlags};

    let mounts = match std::fs::read_to_string("/proc/mounts") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[pid1] cannot read /proc/mounts: {}", e);
            return;
        }
    };

    const SKIP_TYPES: &[&str] = &[
        "proc",
        "sysfs",
        "devtmpfs",
        "devpts",
        "cgroup",
        "cgroup2",
        "tmpfs",
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

    let mut paths: Vec<String> = mounts
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            let mountpoint = parts[1];
            let vfstype = parts[2];
            if mountpoint == "/" {
                return None;
            }
            if SKIP_TYPES.contains(&vfstype) {
                return None;
            }
            Some(mountpoint.to_string())
        })
        .collect();

    // Unmount deepest paths first.
    paths.reverse();

    for path in &paths {
        eprintln!("[pid1] unmounting {}", path);
        if let Err(e) = umount2(path.as_str(), MntFlags::MNT_DETACH) {
            eprintln!("[pid1] umount2 {}: {}", path, e);
        }
    }

    nix::unistd::sync();
    eprintln!("[pid1] filesystems detached and synced");
}

/// Called after executor::run() returns (only when running as PID 1).
///
/// Reads FLINT_ON_EXIT at call time:
///   "halt"   → halt the machine
///   "reboot" → reboot the machine
///   anything else (or unset) → unmount and halt
pub fn supervise_forever() -> ! {
    match std::env::var("FLINT_ON_EXIT").as_deref() {
        Ok("halt") => {
            unmount_all();
            eprintln!("[pid1] FLINT_ON_EXIT=halt — halting");
            let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_HALT_SYSTEM);
        }
        Ok("reboot") => {
            unmount_all();
            eprintln!("[pid1] FLINT_ON_EXIT=reboot — rebooting");
            let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_AUTOBOOT);
        }
        _ => {
            // No explicit exit mode — on a real system this means all services
            // exited on their own. Unmount and halt rather than spin forever.
            unmount_all();
            eprintln!("[pid1] all services done — halting");
            let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_HALT_SYSTEM);
        }
    }

    eprintln!("[pid1] entering zombie-reap loop");
    loop {
        // Reap any zombie child (including orphaned grandchildren).
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, code)) => {
                eprintln!("[pid1] reaped pid {} (exit {})", pid, code);
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                eprintln!("[pid1] reaped pid {} (signal {:?})", pid, sig);
            }
            _ => {
                // No zombie available — sleep before trying again.
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}
