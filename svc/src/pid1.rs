use std::{
    fs::{
        self,
        OpenOptions
    },
    io::Write,
    path::Path,
};

use eyre::{
    Context,
    Result,
    bail,
};
use nix::{
    errno::Errno,
    mount::{
        MsFlags,
        mount,
    },
    sys::{
        reboot::{
            RebootMode,
            reboot,
        },
        wait::{
            WaitPidFlag,
            WaitStatus,
            waitpid,
        },
    },
    unistd::{
        Pid,
        getpid,
        sync,
    },
};
use tokio::signal::unix::{
    SignalKind,
    signal,
};
use tracing::{
    debug,
    error,
    warn,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalAction {
    Poweroff,
    Reboot,
    Halt,
}

pub fn is_pid1() -> bool {
    getpid() == Pid::from_raw(1)
}

/// Reap any exited children. Save alongside Tokio `Child::wait` (already-reaped → ECHILD).
pub async fn reap_orphans() {
    let mut sigchld = match signal(SignalKind::child()) {
        Ok(s) => s,
        Err(err) => {
            error!("failed to register SIGCHLD handler: {err}");
            return;
        }
    };

    loop {
        sigchld.recv().await;
        loop {
            match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => break,
                Ok(status) => debug!("reaped orphan: {status:?}"),
                Err(Errno::ECHILD) => break,
                Err(Errno::EINTR) => continue,
                Err(err) => {
                    warn!("waitpid error while reaping: {err}");
                    break;
                }
            }
        }
    }
}

/// Never returns. Call only when we are PID 1, after services are stopped.
pub fn finalize_as_init(action: FinalAction) -> ! {
    sync();
    let mode = match action {
        FinalAction::Poweroff => RebootMode::RB_POWER_OFF,
        FinalAction::Reboot => RebootMode::RB_AUTOBOOT,
        FinalAction::Halt => RebootMode::RB_HALT_SYSTEM,
    };
    let Err(err) = reboot(mode);
    error!("reboot({mode:?}) failed: {err}");
    // Last resort: hang so the kernel does not panic on PID 1 exit.
    loop {
        std::thread::park();
    }
}

fn console_err(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().write(true).open("/dev/console") {
        let _ = writeln!(f, "rsvc: {msg}");
    }
    eprintln!("rsvc: {msg}");
}

fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).wrap_err_with(|| format!("mkdir {}", path.display()))
}

fn mount_one(
    source: Option<&str>,
    target: &str,
    fstype: Option<&str>,
    flags: MsFlags,
    data: Option<&str>,
) -> Result<()> {
    ensure_dir(Path::new(target))?;
    match mount(source, Path::new(target), fstype, flags, data) {
        Ok(()) => Ok(()),
        Err(Errno::EBUSY) | Err(Errno::EINVAL) | Err(Errno::EEXIST) => Ok(()),
        Err(err) => {
            let msg = format!("mount {target}: {err}");
            console_err(&msg);
            bail!("{msg}");
        }
    }
}

pub fn prepare_early_fs(rundir: &Path, logdir: &Path) -> eyre::Result<()> {
    // Remount root read-write if still ro after switch_root.
    let _ = mount(
        None::<&str>,
        Path::new("/"),
        None::<&str>,
        MsFlags::MS_REMOUNT,
        None::<&str>,
    );

    let common = MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV;
    mount_one(Some("proc"), "/proc", Some("proc"), common, None)?;
    mount_one(Some("sys"), "/sys", Some("sysfs"), common, None)?;
    mount_one(
        Some("dev"),
        "/dev",
        Some("devtmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        Some("mode=0755"),
    )?;
    mount_one(
        Some("tmpfs"),
        "/run",
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755,size=64M"),
    )?;

    ensure_dir(rundir)?;
    ensure_dir(logdir)?;
    Ok(())
}
