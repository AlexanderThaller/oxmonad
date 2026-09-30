//! `oxmonad --restart`: asks the running oxmonad to restart in place, like
//! `xmonad --restart`.
//!
//! The running instance restarts on `SIGUSR2` (the same path as Mod+q,
//! including its `--check-config` safety check). This module finds it: a
//! process of ours named `oxmonad` that, when we know our session, runs in
//! the same session (same `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY`).

use std::{
    env,
    ffi::OsStr,
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
};

use std::{
    thread,
    time::{
        Duration,
        Instant,
    },
};

use rustix::process::{
    Pid,
    Signal,
    getuid,
};

/// The signal that makes a running oxmonad restart in place.
pub const RESTART_SIGNAL: Signal = Signal::USR2;

/// What identifies a process as a running oxmonad in our session.
#[derive(Debug, PartialEq, Eq)]
struct Process {
    pid: u32,
    uid: u32,
    comm: String,
    /// Whether it handles [`RESTART_SIGNAL`]; an oxmonad from before
    /// `--restart` existed would be killed by it instead.
    catches_restart: bool,
    runtime_dir: Option<String>,
    wayland_display: Option<String>,
}

/// Who is asking: ourselves and the session we run in.
#[derive(Debug)]
struct Caller {
    pid: u32,
    uid: u32,
    runtime_dir: Option<String>,
    wayland_display: Option<String>,
}

impl Caller {
    /// Whether `process` is an oxmonad this caller may restart. Without a
    /// session (e.g. over ssh) every oxmonad of the user matches.
    fn targets(&self, process: &Process) -> bool {
        let same =
            |ours: &Option<String>, theirs: &Option<String>| ours.is_none() || ours == theirs;
        process.comm == "oxmonad"
            && process.pid != self.pid
            && process.uid == self.uid
            && same(&self.runtime_dir, &process.runtime_dir)
            && same(&self.wayland_display, &process.wayland_display)
    }
}

/// Signals every running oxmonad in this session to restart and returns
/// the pids signalled.
pub fn restart_running() -> Result<Vec<u32>, String> {
    let non_empty = |name| env::var(name).ok().filter(|v| !v.is_empty());
    let caller = Caller {
        pid: std::process::id(),
        uid: getuid().as_raw(),
        runtime_dir: non_empty("XDG_RUNTIME_DIR"),
        wayland_display: non_empty("WAYLAND_DISPLAY"),
    };
    let entries = fs::read_dir("/proc").map_err(|err| format!("cannot list /proc: {err}"))?;
    let mut signalled = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Some(process) = read_process(&entry.path()) else {
            continue;
        };
        if !caller.targets(&process) {
            continue;
        }
        if !process.catches_restart && !wait_for_handler(&entry.path()) {
            return Err(format!(
                "oxmonad {} is too old for --restart (it would die from SIGUSR2); press Mod+q \
                 once instead",
                process.pid
            ));
        }
        let pid = i32::try_from(process.pid)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| format!("invalid pid {}", process.pid))?;
        match rustix::process::kill_process(pid, RESTART_SIGNAL) {
            Ok(()) => signalled.push(process.pid),
            // It exited meanwhile.
            Err(rustix::io::Errno::SRCH) => {}
            Err(err) => return Err(format!("cannot signal oxmonad {}: {err}", process.pid)),
        }
    }
    Ok(signalled)
}

/// Waits up to [`HANDLER_WAIT`] for the process at `dir` to handle
/// [`RESTART_SIGNAL`]. An oxmonad that just re-executed itself (a restart
/// in progress) installs its handler a moment after starting; an old
/// build never does.
fn wait_for_handler(dir: &Path) -> bool {
    let deadline = Instant::now() + HANDLER_WAIT;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
        let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
        if catches(&status, RESTART_SIGNAL) {
            return true;
        }
    }
    false
}

/// How long `--restart` waits for a restarting oxmonad's signal handler.
const HANDLER_WAIT: Duration = Duration::from_secs(3);

/// Reads what [`Caller::targets`] needs from `/proc/<pid>`; `None` for
/// non-process entries and processes that vanished or aren't readable.
fn read_process(dir: &Path) -> Option<Process> {
    let pid = dir.file_name().and_then(OsStr::to_str)?.parse().ok()?;
    let uid = fs::metadata(dir).ok()?.uid();
    let comm = fs::read_to_string(dir.join("comm")).ok()?;
    let environ = fs::read(dir.join("environ")).unwrap_or_default();
    let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
    Some(Process {
        pid,
        uid,
        comm: comm.trim_end().to_owned(),
        catches_restart: catches(&status, RESTART_SIGNAL),
        runtime_dir: env_value(&environ, "XDG_RUNTIME_DIR"),
        wayland_display: env_value(&environ, "WAYLAND_DISPLAY"),
    })
}

/// Whether the `SigCgt` mask in `/proc/<pid>/status` has a handler for
/// `signal`.
fn catches(status: &str, signal: Signal) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix("SigCgt:"))
        .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
        .is_some_and(|mask| mask & (1 << (signal.as_raw() - 1)) != 0)
}

/// The value of `name` in a NUL-separated `/proc/<pid>/environ`.
fn env_value(environ: &[u8], name: &str) -> Option<String> {
    environ.split(|&b| b == 0).find_map(|entry| {
        let value = entry.strip_prefix(name.as_bytes())?.strip_prefix(b"=")?;
        Some(String::from_utf8_lossy(value).into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(runtime_dir: Option<&str>, display: Option<&str>) -> Caller {
        Caller {
            pid: 10,
            uid: 1000,
            runtime_dir: runtime_dir.map(Into::into),
            wayland_display: display.map(Into::into),
        }
    }

    fn oxmonad(pid: u32) -> Process {
        Process {
            pid,
            uid: 1000,
            comm: "oxmonad".into(),
            catches_restart: true,
            runtime_dir: Some("/run/user/1000".into()),
            wayland_display: Some("wayland-1".into()),
        }
    }

    #[test]
    fn targets_the_oxmonad_of_this_session() {
        let me = caller(Some("/run/user/1000"), Some("wayland-1"));
        assert!(me.targets(&oxmonad(20)));
    }

    #[test]
    fn skips_other_sessions_users_programs_and_itself() {
        let me = caller(Some("/run/user/1000"), Some("wayland-1"));
        let other_display = Process {
            wayland_display: Some("wayland-2".into()),
            ..oxmonad(20)
        };
        let other_runtime = Process {
            runtime_dir: Some("/tmp/test".into()),
            ..oxmonad(20)
        };
        let other_user = Process {
            uid: 0,
            ..oxmonad(20)
        };
        let river = Process {
            comm: "river".into(),
            ..oxmonad(20)
        };
        for process in [other_display, other_runtime, other_user, river, oxmonad(10)] {
            assert!(!me.targets(&process), "{process:?}");
        }
    }

    #[test]
    fn without_a_session_every_oxmonad_of_the_user_matches() {
        let me = caller(None, None);
        let elsewhere = Process {
            wayland_display: Some("wayland-7".into()),
            ..oxmonad(20)
        };
        assert!(me.targets(&elsewhere));
    }

    #[test]
    fn reads_the_caught_signal_mask() {
        // SIGUSR2 is signal 12: bit 11 (0x800).
        let with = "Name:\toxmonad\nSigCgt:\t0000000000014a00\n";
        let without = "Name:\toxmonad\nSigCgt:\t0000000000010000\n";
        assert!(catches(with, Signal::USR2));
        assert!(!catches(without, Signal::USR2));
        assert!(!catches("", Signal::USR2));
    }

    #[test]
    fn this_process_reports_its_mask() {
        let status = fs::read_to_string("/proc/self/status").expect("status");
        assert!(status.contains("SigCgt:"));
    }

    #[test]
    fn reads_environ_values() {
        let environ = b"A=1\0WAYLAND_DISPLAY=wayland-1\0WAYLAND_DISPLAYX=no\0";
        assert_eq!(
            env_value(environ, "WAYLAND_DISPLAY").as_deref(),
            Some("wayland-1")
        );
        assert_eq!(env_value(environ, "XDG_RUNTIME_DIR"), None);
    }
}
