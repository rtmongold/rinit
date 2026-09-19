use nix::{
    errno::Errno,
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
