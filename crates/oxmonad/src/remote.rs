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

/// Reads what [`Caller::targets`] needs from `/proc/<pid>`; `None` for
/// non-process entries and processes that vanished or aren't readable.
fn read_process(dir: &Path) -> Option<Process> {
    let pid = dir.file_name().and_then(OsStr::to_str)?.parse().ok()?;
    let uid = fs::metadata(dir).ok()?.uid();
    let comm = fs::read_to_string(dir.join("comm")).ok()?;
    let environ = fs::read(dir.join("environ")).unwrap_or_default();
    Some(Process {
        pid,
        uid,
        comm: comm.trim_end().to_owned(),
        runtime_dir: env_value(&environ, "XDG_RUNTIME_DIR"),
        wayland_display: env_value(&environ, "WAYLAND_DISPLAY"),
    })
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
    fn reads_environ_values() {
        let environ = b"A=1\0WAYLAND_DISPLAY=wayland-1\0WAYLAND_DISPLAYX=no\0";
        assert_eq!(
            env_value(environ, "WAYLAND_DISPLAY").as_deref(),
            Some("wayland-1")
        );
        assert_eq!(env_value(environ, "XDG_RUNTIME_DIR"), None);
    }
}
