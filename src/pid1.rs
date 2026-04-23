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

    // /dev — tmpfs (udev will populate it)
    std::fs::create_dir_all("/dev").context("create /dev")?;
    mount(
        Some("devtmpfs"),
        "/dev",
        Some("devtmpfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("mount /dev")?;

    eprintln!("[pid1] mounted /proc, /sys, /dev");
    Ok(())
}

/// Called after executor::run() returns (only when running as PID 1).
///
/// Reads FLINT_ON_EXIT at call time:
///   "halt"   → halt the machine
///   "reboot" → reboot the machine
///   anything else (or unset) → reap zombies forever
pub fn supervise_forever() -> ! {
    match std::env::var("FLINT_ON_EXIT").as_deref() {
        Ok("halt") => {
            eprintln!("[pid1] FLINT_ON_EXIT=halt — halting");
            let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_HALT_SYSTEM);
            // If reboot() somehow returns, fall through to the reap loop.
        }
        Ok("reboot") => {
            eprintln!("[pid1] FLINT_ON_EXIT=reboot — rebooting");
            let _ = nix::sys::reboot::reboot(nix::sys::reboot::RebootMode::RB_AUTOBOOT);
        }
        _ => {}
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
