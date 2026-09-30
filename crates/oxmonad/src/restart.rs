//! Restarting in place (`restart`, xmonad's `Mod+q`).
//!
//! The running oxmonad checks that the binary it was started from still
//! works (`--check-config`), saves [`Wm::snapshot`](oxmonad_core::Wm) as
//! JSON in `$XDG_RUNTIME_DIR`, stops window management and re-executes
//! that binary in the same process with [`ENV`] naming the file. river
//! keeps running and keeps the windows; it only sees the window manager
//! disconnect and a new one bind, and announces every window to it again.
//! The new process reads the snapshot back with [`take`].
//!
//! Every file descriptor oxmonad opens is close-on-exec, so the new
//! process starts clean; spawned children stay its children.

use std::{
    env,
    ffi::OsString,
    fs::{
        self,
        OpenOptions,
    },
    io::{
        self,
        Write,
    },
    os::unix::{
        fs::OpenOptionsExt,
        process::CommandExt,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

use oxmonad_core::Snapshot;

/// The environment variable naming the snapshot file to restore.
pub const ENV: &str = "OXMONAD_RESTORE";

/// How this process was started, to start the same way again.
#[derive(Debug, Clone)]
pub struct Exe {
    /// The binary, resolved at startup: a rebuild replaces the file at this
    /// path, and the restart picks up the new one.
    path: PathBuf,
    /// `argv[0]`.
    arg0: OsString,
    /// The other arguments.
    args: Vec<OsString>,
}

impl Exe {
    /// The running binary and its arguments.
    pub fn current() -> io::Result<Self> {
        let path = env::current_exe()?;
        // Linux marks a replaced binary `… (deleted)`.
        let path = match path.to_str().and_then(|p| p.strip_suffix(" (deleted)")) {
            Some(original) => PathBuf::from(original),
            None => path,
        };
        let mut argv = env::args_os();
        let first = argv.next().unwrap_or_else(|| path.clone().into_os_string());
        Ok(Self {
            path,
            arg0: first,
            args: argv.collect(),
        })
    }

    /// Runs the binary with `--check-config`: whether it starts and
    /// accepts the configuration. Blocks until it exits.
    pub fn check(&self) -> Result<(), String> {
        let output = Command::new(&self.path)
            .arg0(&self.arg0)
            .args(&self.args)
            .arg("--check-config")
            .env_remove(ENV)
            .output()
            .map_err(|err| format!("cannot run {}: {err}", self.path.display()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} --check-config failed ({}): {}",
                self.path.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    /// Replaces this process with the binary, restoring `snapshot`. Only
    /// returns if that fails.
    pub fn exec(&self, snapshot: &Path) -> io::Error {
        Command::new(&self.path)
            .arg0(&self.arg0)
            .args(&self.args)
            .env(ENV, snapshot)
            .exec()
    }
}

/// Writes `snapshot` to a private file in `$XDG_RUNTIME_DIR` and returns
/// its path.
pub fn save(snapshot: &Snapshot) -> io::Result<PathBuf> {
    let dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(env::temp_dir, PathBuf::from);
    save_in(&dir, snapshot)
}

fn save_in(dir: &Path, snapshot: &Snapshot) -> io::Result<PathBuf> {
    let path = dir.join(format!("oxmonad-restart-{}.json", std::process::id()));
    let json = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&json)?;
    Ok(path)
}

/// Removes a snapshot file that will not be restored.
pub fn discard(path: &Path) {
    if let Err(err) = fs::remove_file(path) {
        tracing::warn!(path = %path.display(), %err, "cannot remove the restart snapshot");
    }
}

/// The snapshot the previous process left in [`ENV`], if any. The file is
/// removed; an unreadable or invalid snapshot is logged and ignored.
pub fn take() -> Option<Snapshot> {
    let path = PathBuf::from(env::var_os(ENV).filter(|p| !p.is_empty())?);
    let read = fs::read(&path);
    discard(&path);
    let parsed = read
        .map_err(|err| err.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|err| err.to_string()));
    match parsed {
        Ok(snapshot) => Some(snapshot),
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "ignoring the restart snapshot");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use oxmonad_core::{
        Wm,
        config::Config,
    };

    use super::*;

    #[test]
    fn saved_snapshots_read_back() {
        let wm = Wm::new(Config::defaults());
        let snapshot = wm.snapshot(|w| Some(w.0.to_string()));
        let dir = env::temp_dir().join(format!("oxmonad-restart-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = save_in(&dir, &snapshot).expect("saved");
        let json = fs::read(&path).expect("readable");
        discard(&path);
        assert!(!path.exists());
        fs::remove_dir(&dir).expect("temp dir removed");
        let read: Snapshot = serde_json::from_slice(&json).expect("valid JSON");
        assert_eq!(read, snapshot);
    }
}
